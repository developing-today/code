#[cfg(test)]
use crate::world::*;
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    #[derive(Debug)]
    struct SplitClock {
        wall_ms: AtomicU64,
        monotonic_ms: AtomicU64,
        monotonic_origin: Instant,
    }

    impl Clock for SplitClock {
        fn now_ms(&self) -> u64 {
            self.wall_ms.load(Ordering::SeqCst)
        }

        fn now_instant(&self) -> Instant {
            self.monotonic_origin + Duration::from_millis(self.monotonic_ms.load(Ordering::SeqCst))
        }
    }

    fn world(retained_events: usize) -> WorldCore {
        WorldCore::new(
            "test-world",
            WorldLimits {
                retained_events,
                ..WorldLimits::default()
            },
        )
        .unwrap()
    }

    fn join(core: &mut WorldCore, name: &str) -> (Participant, JoinCapability) {
        core.issue_capability(name, WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap()
    }

    #[test]
    fn test_form_result_system_journal_replay_spends_submitter_budget() {
        let mut core = world(8);
        core.active_module_hash = Some("module".to_owned());
        let (participant, _) = core
            .issue_capability(
                "Ada",
                WorldScopes::GUEST,
                CapabilityBounds {
                    uses: Some(1),
                    ..CapabilityBounds::default()
                },
            )
            .unwrap();
        let mut entries = core.checkpoint_entries("{}".to_owned()).unwrap();
        entries.push(JournalEntry::System {
            event: WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: serde_json::json!({
                        "cap": "form.result",
                        "form_id": "form-1",
                        "participant_id": participant.id,
                        "action": "submit",
                        "values": {"name": "Ada"},
                    })
                    .to_string(),
                },
            },
        });

        let restored = WorldCore::restore(
            "test-world",
            WorldLimits {
                retained_events: 8,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap();
        assert_eq!(restored.core.uses_left(participant.id), Some(0));
    }

    #[derive(Clone, Default)]
    struct TransferJournal(Arc<std::sync::Mutex<Vec<JournalEntry>>>);

    impl TransferJournal {
        fn for_world(world_id: &str) -> Self {
            Self(Arc::new(std::sync::Mutex::new(vec![
                JournalEntry::Created {
                    world_id: world_id.to_owned(),
                    version: JOURNAL_VERSION_REGION_HANDOFF,
                },
            ])))
        }

        fn entries(&self) -> Vec<JournalEntry> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Append like `FileJournal`: raise the `Created` header to the version
    /// the entry requires, so restore sees the journal a writer would leave.
    pub(super) fn push_bumped(entries: &mut Vec<JournalEntry>, entry: &JournalEntry) {
        let required = journal_version_required_by(entry);
        if let Some(JournalEntry::Created { version, .. }) = entries.first_mut() {
            if *version < required {
                *version = required;
            }
        }
        entries.push(entry.clone());
    }

    impl WorldJournal for TransferJournal {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            push_bumped(&mut self.0.lock().unwrap(), entry);
            Ok(())
        }
    }

    struct TransferProgram {
        payload: Vec<u8>,
        fail_apply: bool,
        journal: TransferJournal,
        saw_durable_export_before_apply: Arc<std::sync::Mutex<bool>>,
        applied_events: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl WorldProgram for TransferProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                let envelope: serde_json::Value = serde_json::from_str(event)?;
                self.applied_events.lock().unwrap().push(
                    envelope["app_event"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                );
                let durable = self.journal.entries().iter().any(|entry| {
                    if envelope["direction"] == "import" {
                        matches!(entry, JournalEntry::EntityImported { .. })
                    } else {
                        matches!(entry, JournalEntry::EntityExported { .. })
                    }
                });
                *self.saw_durable_export_before_apply.lock().unwrap() = durable;
                ensure!(durable, "transfer event applied before it was journaled");
                if self.fail_apply {
                    bail!("injected transfer-apply failure");
                }
            }
            Ok(())
        }

        fn records(&mut self) -> Result<Option<String>> {
            let applied = self.applied_events.lock().unwrap().clone();
            Ok(Some(serde_json::json!({ "applied": applied }).to_string()))
        }

        fn prepare_entity_transfer(
            &self,
            request: &EntityTransferRequest,
        ) -> Result<EntityTransferPlan> {
            match request {
                EntityTransferRequest::Export { .. } => Ok(EntityTransferPlan::Export {
                    payload_schema: "test/v1".to_owned(),
                    payload: self.payload.clone(),
                    app_event: "remove-entity".to_owned(),
                }),
                EntityTransferRequest::Import {
                    payload_schema,
                    payload,
                    payload_digest,
                    ..
                } => {
                    ensure!(
                        payload_schema == "test/v1",
                        "unsupported payload schema {payload_schema:?}"
                    );
                    ensure!(
                        *payload_digest == hex_encode(blake3::hash(payload).as_bytes()),
                        "import payload digest does not match"
                    );
                    Ok(EntityTransferPlan::Import {
                        app_event: "add-entity".to_owned(),
                    })
                }
            }
        }
    }

    fn transfer_actor(
        payload: Vec<u8>,
        fail_apply: bool,
    ) -> (
        WorldActor,
        JoinCapability,
        TransferJournal,
        Arc<std::sync::Mutex<bool>>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        let mut core = WorldCore::new(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        core.active_module_hash = Some("module-hash".to_owned());
        let (_, token) = join(&mut core, "source-user");
        let journal = TransferJournal::for_world("source");
        let saw_durable_export_before_apply = Arc::new(std::sync::Mutex::new(false));
        let applied_events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let program = TransferProgram {
            payload,
            fail_apply,
            journal: journal.clone(),
            saw_durable_export_before_apply: saw_durable_export_before_apply.clone(),
            applied_events: applied_events.clone(),
        };
        let actor = WorldActor::for_test(core, Box::new(program), Some(Box::new(journal.clone())));
        (
            actor,
            token,
            journal,
            saw_durable_export_before_apply,
            applied_events,
        )
    }

    async fn export(actor: &mut WorldActor) -> Result<HandoffTransferState> {
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ExportEntity {
                handoff_id: "ab".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-a".to_owned(),
                target_region: "region-b".to_owned(),
                prior_owner_epoch: 7,
                reply,
            })
            .await;
        response.await.unwrap()
    }

    #[tokio::test]
    async fn export_is_journaled_before_apply_replayed_and_fences_later_writes() {
        let (mut actor, token, journal, saw_durable, applied) =
            transfer_actor(b"entity".to_vec(), false);
        let receipt = export(&mut actor).await.unwrap();
        assert_eq!(receipt.payload.as_deref(), Some(b"entity".as_slice()));
        assert!(*saw_durable.lock().unwrap());
        assert!(actor.core.entity_is_fenced("entity-1"));

        let entries = journal.entries();
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::EntityExported { .. })
        ));
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers(), &[receipt.clone()]);
        assert!(restored.core.entity_is_fenced("entity-1"));
        assert_eq!(restored.replay, vec![receipt.event.clone()]);
        let replayed = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut replay_program = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: replayed.clone(),
        };
        replay_program.update(&restored.replay[0]).unwrap();
        assert_eq!(*replayed.lock().unwrap(), *applied.lock().unwrap());

        let before = journal.entries().len();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::EntityInput {
                token,
                entity_ref: "entity-1".to_owned(),
                input: b"mutate".to_vec(),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_err());
        assert_eq!(journal.entries().len(), before);
    }

    #[tokio::test]
    async fn oversized_export_is_refused_before_a_durable_fence() {
        let payload = vec![0; MAX_ENTITY_TRANSFER_PAYLOAD_BYTES + 1];
        let (mut actor, _, journal, _, _) = transfer_actor(payload, false);
        assert!(export(&mut actor).await.is_err());
        assert!(actor.core.handoff_transfers().is_empty());
        assert!(
            !journal
                .entries()
                .iter()
                .any(|entry| matches!(entry, JournalEntry::EntityExported { .. }))
        );
    }

    #[tokio::test]
    async fn failed_post_append_apply_withholds_receipt_and_keeps_fence() {
        let (mut actor, _, journal, saw_durable, _) = transfer_actor(b"entity".to_vec(), true);
        assert!(export(&mut actor).await.is_err());
        assert!(*saw_durable.lock().unwrap());
        assert!(actor.core.entity_is_fenced("entity-1"));
        assert!(!actor.program_healthy);
        assert!(matches!(
            journal.entries().last(),
            Some(JournalEntry::EntityExported { .. })
        ));
    }

    #[tokio::test]
    async fn export_exact_retry_returns_the_stored_receipt() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), false);
        let receipt = export(&mut actor).await.unwrap();
        let entries_after_first = journal.entries().len();
        let retry = export(&mut actor).await.unwrap();
        assert_eq!(retry, receipt);
        assert_eq!(journal.entries().len(), entries_after_first);

        // A conflicting reuse of the handoff ID is not a retry.
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ExportEntity {
                handoff_id: "ab".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-a".to_owned(),
                target_region: "region-c".to_owned(),
                prior_owner_epoch: 7,
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_err());
    }

    async fn import(actor: &mut WorldActor) -> Result<HandoffTransferState> {
        import_payload(actor, b"entity".to_vec()).await
    }

    async fn import_payload(
        actor: &mut WorldActor,
        payload: Vec<u8>,
    ) -> Result<HandoffTransferState> {
        let payload_digest = hex_encode(blake3::hash(&payload).as_bytes());
        import_with(actor, "module-hash", "test/v1", payload, payload_digest).await
    }

    async fn import_with(
        actor: &mut WorldActor,
        program_module_hash: &str,
        payload_schema: &str,
        payload: Vec<u8>,
        payload_digest: String,
    ) -> Result<HandoffTransferState> {
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ImportEntity {
                handoff_id: "cd".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-a".to_owned(),
                target_region: "region-b".to_owned(),
                prior_owner_epoch: 7,
                program_module_hash: program_module_hash.to_owned(),
                payload_schema: payload_schema.to_owned(),
                payload,
                payload_digest,
                reply,
            })
            .await;
        response.await.unwrap()
    }

    #[tokio::test]
    async fn import_is_journaled_before_apply_and_replayed() {
        let (mut actor, _, journal, saw_durable, applied) =
            transfer_actor(b"entity".to_vec(), false);
        let receipt = import(&mut actor).await.unwrap();
        assert_eq!(receipt.direction, HandoffDirection::Import);
        assert_eq!(receipt.new_owner_epoch, Some(8));
        assert_eq!(receipt.payload, None);
        assert!(*saw_durable.lock().unwrap());
        assert_eq!(*applied.lock().unwrap(), vec!["add-entity".to_owned()]);
        assert!(matches!(
            journal.entries().last(),
            Some(JournalEntry::EntityImported { .. })
        ));

        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            journal.entries(),
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers(), &[receipt.clone()]);
        assert_eq!(restored.replay, vec![receipt.event.clone()]);
    }

    #[tokio::test]
    async fn import_exact_retry_returns_the_stored_receipt_without_reapplying() {
        let (mut actor, _, journal, _, applied) = transfer_actor(b"entity".to_vec(), false);
        let receipt = import(&mut actor).await.unwrap();
        let entries_after_first = journal.entries().len();
        let retry = import(&mut actor).await.unwrap();
        assert_eq!(retry, receipt);
        assert_eq!(journal.entries().len(), entries_after_first);
        assert_eq!(applied.lock().unwrap().len(), 1);

        // A conflicting reuse of the handoff ID is not a retry: it fails
        // closed and the stored receipt is never overwritten.
        assert!(
            import_payload(&mut actor, b"different".to_vec())
                .await
                .is_err()
        );
        assert_eq!(journal.entries().len(), entries_after_first);
        assert_eq!(actor.core.handoff_transfers(), &[receipt]);
    }

    #[tokio::test]
    async fn import_dedupes_after_checkpoint_compaction() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), false);
        let receipt = import(&mut actor).await.unwrap();
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            journal.entries(),
        )
        .unwrap();
        let mut core = restored.core;
        core.active_module_hash = Some("module-hash".to_owned());
        let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            compacted.clone(),
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers(), &[receipt.clone()]);

        // A retry after compaction observes the receipt: it neither
        // journals nor applies a second import.
        let compacted_journal = TransferJournal(Arc::new(std::sync::Mutex::new(compacted)));
        let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
        let program = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: compacted_journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: applied.clone(),
        };
        let mut actor = WorldActor::for_test(
            restored.core,
            Box::new(program),
            Some(Box::new(compacted_journal.clone())),
        );
        let retry = import(&mut actor).await.unwrap();
        assert_eq!(retry, receipt);
        assert!(applied.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn import_validates_the_envelope_before_any_mutation() {
        let (mut actor, _, journal, _, applied) = transfer_actor(b"entity".to_vec(), false);
        let before = journal.entries().len();
        let good_digest = hex_encode(blake3::hash(b"entity").as_bytes());

        // A digest that does not match the payload.
        assert!(
            import_with(
                &mut actor,
                "module-hash",
                "test/v1",
                b"entity".to_vec(),
                "00".repeat(32),
            )
            .await
            .is_err()
        );
        // A module hash the destination does not run.
        assert!(
            import_with(
                &mut actor,
                "other-module",
                "test/v1",
                b"entity".to_vec(),
                good_digest.clone(),
            )
            .await
            .is_err()
        );
        // An unsupported payload schema (the program refuses in prepare).
        assert!(
            import_with(
                &mut actor,
                "module-hash",
                "test/v2",
                b"entity".to_vec(),
                good_digest,
            )
            .await
            .is_err()
        );
        // An oversized payload.
        assert!(
            import_payload(&mut actor, vec![0; MAX_ENTITY_TRANSFER_PAYLOAD_BYTES + 1])
                .await
                .is_err()
        );

        assert_eq!(journal.entries().len(), before);
        assert!(actor.core.handoff_transfers().is_empty());
        assert!(applied.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_post_append_import_apply_withholds_receipt_and_keeps_the_dedupe_record() {
        let (mut actor, _, journal, saw_durable, _) = transfer_actor(b"entity".to_vec(), true);
        assert!(import(&mut actor).await.is_err());
        assert!(*saw_durable.lock().unwrap());
        assert!(!actor.program_healthy);
        assert!(matches!(
            journal.entries().last(),
            Some(JournalEntry::EntityImported { .. })
        ));
        assert_eq!(actor.core.handoff_transfers().len(), 1);
        assert_eq!(
            actor.core.handoff_transfers()[0].direction,
            HandoffDirection::Import
        );
    }

    #[tokio::test]
    async fn imported_back_entity_loses_its_fence_and_can_cross_again() {
        let (mut actor, token, journal, _, _) = transfer_actor(b"entity".to_vec(), false);
        let exported = export(&mut actor).await.unwrap();
        assert!(actor.core.entity_is_fenced("entity-1"));

        // The entity crosses back B -> A under a fresh handoff ID with the
        // owner epoch advanced (design #2818 §3 permits later crossings).
        let payload = b"entity".to_vec();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ImportEntity {
                handoff_id: "ef".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-b".to_owned(),
                target_region: "region-a".to_owned(),
                prior_owner_epoch: 7,
                program_module_hash: "module-hash".to_owned(),
                payload_schema: "test/v1".to_owned(),
                payload_digest: hex_encode(blake3::hash(&payload).as_bytes()),
                payload,
                reply,
            })
            .await;
        let imported = response.await.unwrap().unwrap();
        assert_eq!(imported.direction, HandoffDirection::Import);

        // The historical export receipt stays in the ledger for dedupe,
        // but the fence derives from the latest transfer: the entity is
        // writable again.
        assert_eq!(actor.core.handoff_transfers().len(), 2);
        assert!(!actor.core.entity_is_fenced("entity-1"));
        // The fence gate itself accepts writes again (checked at prepare
        // level so the test journal keeps only transfer entries, like the
        // restore-based tests above).
        assert!(
            actor
                .core
                .prepare_entity_input(&token, "entity-1", b"mutate")
                .is_ok()
        );

        // And it can cross out again under another fresh handoff ID.
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ExportEntity {
                handoff_id: "12".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-a".to_owned(),
                target_region: "region-b".to_owned(),
                prior_owner_epoch: 8,
                reply,
            })
            .await;
        let re_exported = response.await.unwrap().unwrap();
        assert!(actor.core.entity_is_fenced("entity-1"));

        // The whole round trip restores: both crossings replay in order
        // and the restored fence follows the latest transfer.
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            journal.entries(),
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers().len(), 3);
        assert_eq!(
            restored.replay,
            vec![
                exported.event.clone(),
                imported.event.clone(),
                re_exported.event.clone()
            ]
        );
        assert!(restored.core.entity_is_fenced("entity-1"));
    }

    #[tokio::test]
    async fn failed_import_replay_requirement_survives_program_replacement() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), true);
        assert!(import(&mut actor).await.is_err());
        assert!(!actor.program_healthy);
        assert_eq!(actor.pending_transfer_replays.len(), 1);

        // Reinstalling the same module must not clear the requirement:
        // the replacement program replays the pending import as part of
        // the install, and only then is the world healthy again.
        let applied_after_install = Arc::new(std::sync::Mutex::new(Vec::new()));
        let replacement = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: applied_after_install.clone(),
        };
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::InstallProgram {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                installed_by: None,
                trust: None,
                program: Box::new(replacement),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_ok());
        assert!(actor.pending_transfer_replays.is_empty());
        assert!(actor.program_healthy);
        assert_eq!(
            *applied_after_install.lock().unwrap(),
            vec!["add-entity".to_owned()],
            "the replacement program applied the pending import exactly once"
        );

        // Now the exact retry may return the stored receipt — and it must
        // not apply the import a second time.
        let receipt = import(&mut actor).await.unwrap();
        assert_eq!(receipt.direction, HandoffDirection::Import);
        assert_eq!(
            *applied_after_install.lock().unwrap(),
            vec!["add-entity".to_owned()]
        );
    }

    #[tokio::test]
    async fn successful_import_replays_into_replacement_on_live_install() {
        let (mut actor, _, journal, _, applied_first) = transfer_actor(b"entity".to_vec(), false);
        let receipt = import(&mut actor).await.unwrap();
        assert_eq!(receipt.direction, HandoffDirection::Import);
        assert_eq!(
            *applied_first.lock().unwrap(),
            vec!["add-entity".to_owned()]
        );
        assert!(actor.pending_transfer_replays.is_empty());
        assert!(actor.program_healthy);

        // A live reinstall of the same module must hand the replacement
        // the same transfer history a journal restore would: restore
        // retains the successful import across the install, so the live
        // replacement applies it too - exactly once - instead of losing
        // the entity until a restart resurrects it (#3902).
        let applied_after_install = Arc::new(std::sync::Mutex::new(Vec::new()));
        let replacement = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: applied_after_install.clone(),
        };
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::InstallProgram {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                installed_by: None,
                trust: None,
                program: Box::new(replacement),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_ok());
        assert!(actor.program_healthy);
        assert!(actor.pending_transfer_replays.is_empty());
        assert_eq!(
            *applied_after_install.lock().unwrap(),
            vec!["add-entity".to_owned()],
            "the replacement program applied the retained import exactly once"
        );

        // The restore path agrees: the same journal replays the import
        // into a restored program as well.
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            journal.entries(),
        )
        .unwrap();
        assert_eq!(restored.replay, vec![receipt.event.clone()]);
    }

    #[tokio::test]
    async fn checkpoint_carried_transfer_replays_after_install_on_restore() {
        let (mut actor, _, _, _, _) = transfer_actor(b"entity".to_vec(), false);
        let receipt = import(&mut actor).await.unwrap();

        // Compact the import into a checkpoint: the transfer now sits in
        // the checkpoint ledger (transfers), and its event never enters a
        // restored replay on its own. A program install journaled after
        // the checkpoint must still hand the transfer to the replacement
        // when the journal is restored (#3902).
        let install = WorldEvent {
            sequence: receipt.event.sequence + 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                trust: None,
            },
        };
        let restored = WorldCore::restore(
            "source",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
            [
                JournalEntry::Created {
                    world_id: "source".to_owned(),
                    version: JOURNAL_VERSION_REGION_HANDOFF,
                },
                JournalEntry::Checkpoint {
                    sequence: receipt.event.sequence,
                    module_hash: "module-hash".to_owned(),
                    seed: 0,
                    provenance: None,
                    execution_profile: None,
                    caps_ceiling: None,
                    schedules: None,
                    kv: None,
                    blobs: None,
                    transfers: Some(vec![receipt.clone()]),
                    lease: None,
                    snapshot: "snapshot".to_owned(),
                    events: Vec::new(),
                    admin_history: None,
                    open_presence: None,
                },
                JournalEntry::Committed { event: install },
            ],
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers().len(), 1);
        assert_eq!(
            restored.replay,
            vec![receipt.event.clone()],
            "the checkpoint-carried import replays into the replacement after restore"
        );
    }

    #[tokio::test]
    async fn install_publishes_records_recomputed_after_transfer_replay() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), true);
        assert!(import(&mut actor).await.is_err());
        assert_eq!(actor.pending_transfer_replays.len(), 1);

        // The replacement recovers the pending import during install;
        // the records published for the install must describe the
        // replacement AFTER that replay, not the empty pre-replay state
        // computed before it (#3902).
        let applied_after_install = Arc::new(std::sync::Mutex::new(Vec::new()));
        let replacement = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: applied_after_install.clone(),
        };
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::InstallProgram {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                installed_by: None,
                trust: None,
                program: Box::new(replacement),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_ok());
        assert!(actor.program_healthy);
        let published = actor.records_sender.borrow().1.clone();
        assert_eq!(
            published["applied"],
            serde_json::json!(["add-entity"]),
            "published records reflect the import the replacement just applied"
        );
    }

    #[tokio::test]
    async fn install_reports_failure_when_transfer_replay_still_fails() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), true);
        assert!(import(&mut actor).await.is_err());
        assert_eq!(actor.pending_transfer_replays.len(), 1);

        // A replacement that also rejects the pending import must not be
        // acknowledged as a successful install: the world is still
        // unhealthy and the receipt still withheld, so the install
        // result (and its audit record) reports the replay failure
        // (#3902).
        let failing_replacement = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: true,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::InstallProgram {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                installed_by: None,
                trust: None,
                program: Box::new(failing_replacement),
                reply,
            })
            .await;
        let error = response.await.unwrap().unwrap_err();
        assert!(
            format!("{error:#}").contains("rejected a retained transfer replay"),
            "the install result names the replay failure: {error:#}"
        );
        assert!(!actor.program_healthy);
        assert_eq!(actor.pending_transfer_replays.len(), 1);

        // Recovery is still possible: a program that accepts the replay
        // installs successfully and drains the requirement.
        let good_replacement = TransferProgram {
            payload: b"entity".to_vec(),
            fail_apply: false,
            journal: journal.clone(),
            saw_durable_export_before_apply: Arc::new(std::sync::Mutex::new(false)),
            applied_events: Arc::new(std::sync::Mutex::new(Vec::new())),
        };
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::InstallProgram {
                module_hash: "module-hash".to_owned(),
                seed: 0,
                provenance: None,
                installed_by: None,
                trust: None,
                program: Box::new(good_replacement),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_ok());
        assert!(actor.program_healthy);
        assert!(actor.pending_transfer_replays.is_empty());
    }

    #[tokio::test]
    async fn stale_epoch_import_cannot_supersede_an_export_fence() {
        let (mut actor, _, journal, _, _) = transfer_actor(b"entity".to_vec(), false);
        let exported = export(&mut actor).await.unwrap();
        assert!(actor.core.entity_is_fenced("entity-1"));

        // A returning import under a fresh handoff ID but a STALE prior
        // epoch (6, while the export left ownership at 7) must be
        // rejected: recording it would make it the latest transfer and
        // silently clear the fence (#3902).
        let payload = b"entity".to_vec();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ImportEntity {
                handoff_id: "34".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-b".to_owned(),
                target_region: "region-a".to_owned(),
                prior_owner_epoch: 6,
                program_module_hash: "module-hash".to_owned(),
                payload_schema: "test/v1".to_owned(),
                payload_digest: hex_encode(blake3::hash(&payload).as_bytes()),
                payload,
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_err());
        assert!(actor.core.entity_is_fenced("entity-1"));
        assert_eq!(actor.core.handoff_transfers().len(), 1);

        // The same invariant holds at journal restoration: a journal
        // carrying the stale import after the export is refused.
        let stale_event = {
            let payload = b"entity".to_vec();
            let envelope = serde_json::json!({
                "protocol": "world.entity-transfer.v1",
                "direction": "import",
                "app_event": "add-entity",
                "payload_hex": hex_encode(&payload),
                "payload_digest": hex_encode(blake3::hash(&payload).as_bytes()),
            });
            WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: envelope.to_string(),
                },
            }
        };
        let mut entries = journal.entries();
        entries.push(JournalEntry::EntityImported {
            handoff_id: "34".repeat(16),
            entity_ref: "entity-1".to_owned(),
            source_region: "region-b".to_owned(),
            target_region: "region-a".to_owned(),
            prior_owner_epoch: 6,
            new_owner_epoch: 7,
            program_module_hash: "module-hash".to_owned(),
            payload_schema: "test/v1".to_owned(),
            payload_digest: exported.payload_digest.clone(),
            destination_event: stale_event,
        });
        assert!(
            WorldCore::restore(
                "source",
                WorldLimits {
                    region_sharded: true,
                    ..WorldLimits::default()
                },
                entries,
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn advanced_epoch_return_import_is_accepted() {
        let (mut actor, _, _, _, _) = transfer_actor(b"entity".to_vec(), false);
        export(&mut actor).await.unwrap();
        assert!(actor.core.entity_is_fenced("entity-1"));

        // The real round trip: the entity was imported on the other shard
        // (advancing ownership 7 -> 8) and exported back from there, so
        // the return import chains from epoch 8 even though this shard
        // last saw it at 7. That is a legitimate advance, not staleness
        // (#3902).
        let payload = b"entity".to_vec();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ImportEntity {
                handoff_id: "56".repeat(16),
                entity_ref: "entity-1".to_owned(),
                source_region: "region-b".to_owned(),
                target_region: "region-a".to_owned(),
                prior_owner_epoch: 8,
                program_module_hash: "module-hash".to_owned(),
                payload_schema: "test/v1".to_owned(),
                payload_digest: hex_encode(blake3::hash(&payload).as_bytes()),
                payload,
                reply,
            })
            .await;
        let imported = response.await.unwrap().unwrap();
        assert_eq!(imported.new_owner_epoch, Some(9));
        assert!(!actor.core.entity_is_fenced("entity-1"));
    }

    #[tokio::test]
    async fn entity_input_on_a_read_only_replica_never_reaches_the_program() {
        let (mut actor, token, _, _, applied) = transfer_actor(b"entity".to_vec(), false);
        actor.read_only = true;
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::EntityInput {
                token,
                entity_ref: "entity-1".to_owned(),
                input: b"mutate".to_vec(),
                reply,
            })
            .await;
        assert!(response.await.unwrap().is_err());
        assert!(applied.lock().unwrap().is_empty());
    }

    #[test]
    fn bound_author_participant_cannot_use_unsigned_entity_input() {
        use p2panda_core::{Hash, SigningKey};

        let mut core = WorldCore::new(
            "sharded-bound",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, token) = join(&mut core, "bound-user");
        let key = SigningKey::from_bytes(Hash::digest(b"bound user").as_bytes());
        core.bind_author_key(&token, *key.verifying_key().as_bytes())
            .unwrap();
        assert!(
            core.prepare_entity_input(&token, "entity-1", b"scoped")
                .is_err()
        );
    }

    #[test]
    fn region_sharded_world_rejects_signed_unscoped_input() {
        let mut core = WorldCore::new(
            "sharded-signed",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, token) = join(&mut core, "user");
        let result = core.prepare_input_signed(&token, b"legacy", b"not-a-signed-move");
        let error = result.err().expect("signed unscoped input must be refused");
        assert!(
            format!("{error:#}").contains("entity-addressed"),
            "unexpected refusal: {error:#}"
        );
    }

    #[test]
    fn region_sharded_world_rejects_unscoped_input() {
        let mut core = WorldCore::new(
            "sharded",
            WorldLimits {
                region_sharded: true,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, token) = join(&mut core, "user");
        assert!(core.prepare_input(&token, b"legacy").is_err());
        assert!(
            core.prepare_entity_input(&token, "entity-1", b"scoped")
                .is_ok()
        );
    }

    fn sample_source_provenance() -> ModuleProvenance {
        ModuleProvenance::SourceBuild(SourceBuildProvenance {
            schema_version: 1,
            compiler_blake3: "compiler".to_owned(),
            compiler_version: "roc-test 0.0.0".to_owned(),
            platform_blake3: "platform".to_owned(),
            source_blake3: "source".to_owned(),
            target: "wasm32".to_owned(),
            arguments: vec!["build".to_owned(), "main.roc".to_owned()],
        })
    }

    #[test]
    fn provenance_is_optional_and_old_install_events_keep_their_wire_shape() {
        let old = serde_json::json!({
            "sequence": 1,
            "participant_id": 0,
            "kind": { "kind": "program_installed", "data": { "module_hash": "h", "seed": 3 } }
        });
        let parsed: WorldEvent = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            parsed.kind,
            WorldEventKind::ProgramInstalled {
                provenance: None,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(parsed).unwrap(), old);

        let event = WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 3,
                provenance: Some(sample_source_provenance()),
                trust: None,
            },
        };
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serde_json::from_value::<WorldEvent>(encoded).unwrap(),
            event
        );
    }

    #[test]
    fn checkpoint_defaults_missing_provenance_and_round_trips_recorded_provenance() {
        let old = serde_json::json!({
            "entry": "checkpoint",
            "sequence": 0,
            "module_hash": "h",
            "seed": 1,
            "snapshot": "s",
            "events": []
        });
        let parsed: JournalEntry = serde_json::from_value(old.clone()).unwrap();
        assert!(matches!(
            parsed,
            JournalEntry::Checkpoint {
                provenance: None,
                execution_profile: None,
                ..
            }
        ));
        assert_eq!(serde_json::to_value(parsed).unwrap(), old);
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION,
                },
                serde_json::from_value(old).unwrap(),
            ],
        )
        .unwrap();
        assert_eq!(
            restored.core.execution_profile(),
            crate::world_limits::WorldExecutionProfile::default()
        );
    }

    #[test]
    fn restore_checkpoint_sequence_does_not_wrap_at_u64_max() {
        fn system_event(sequence: u64) -> WorldEvent {
            // Checkpoint `events` are the retained participant stream: host
            // System events are refused there by the #242 contract (they live
            // in admin_history), so this sequence-boundary test uses Chat.
            WorldEvent {
                sequence,
                participant_id: 0,
                kind: WorldEventKind::Chat("checkpoint boundary test".to_owned()),
            }
        }

        fn checkpoint(sequence: u64, events: Vec<WorldEvent>) -> JournalEntry {
            JournalEntry::Checkpoint {
                sequence,
                module_hash: "h".to_owned(),
                seed: 1,
                provenance: None,
                execution_profile: None,
                caps_ceiling: None,
                schedules: None,
                kv: None,
                blobs: None,
                transfers: None,
                lease: None,
                snapshot: "s".to_owned(),
                events,
                open_presence: None,
                admin_history: None,
            }
        }

        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION,
                },
                checkpoint(0, vec![system_event(u64::MAX), system_event(0)]),
            ],
        )
        .err()
        .expect("wrapped checkpoint sequence must be rejected");
        assert!(
            error
                .to_string()
                .contains("journal checkpoint events are out of order"),
            "{error:#}"
        );

        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION,
                },
                checkpoint(
                    u64::MAX,
                    vec![system_event(u64::MAX - 1), system_event(u64::MAX)],
                ),
            ],
        )
        .unwrap();
        assert_eq!(restored.core.sequence, u64::MAX);
    }

    #[test]
    fn restore_committed_sequence_does_not_wrap_at_u64_max() {
        // #8370 (design #4000 decision 2): the Committed arm's succession
        // predicate must be checked_add. With saturating_add, a core
        // already at u64::MAX would accept a further committed event with
        // sequence MAX — the saturated successor equals the current
        // sequence, so the "advance by one" check passes without
        // advancing, and repeated MAX events would each pass it again.
        // Oracle: the succession contract itself — after a checkpoint at
        // MAX, no committed event can legitimately follow.
        fn chat(sequence: u64) -> WorldEvent {
            WorldEvent {
                sequence,
                participant_id: 0,
                kind: WorldEventKind::Chat("committed boundary test".to_owned()),
            }
        }

        let journal = |committed: WorldEvent| {
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION,
                },
                JournalEntry::Checkpoint {
                    sequence: u64::MAX,
                    module_hash: "h".to_owned(),
                    seed: 1,
                    provenance: None,
                    execution_profile: None,
                    caps_ceiling: None,
                    schedules: None,
                    kv: None,
                    blobs: None,
                    transfers: None,
                    lease: None,
                    snapshot: "s".to_owned(),
                    events: vec![chat(u64::MAX - 1), chat(u64::MAX)],
                    open_presence: None,
                    admin_history: None,
                },
                JournalEntry::Committed { event: committed },
            ]
        };

        let error = WorldCore::restore("w", WorldLimits::default(), journal(chat(u64::MAX)))
            .err()
            .expect("a committed event at u64::MAX must not pass as the successor of MAX");
        assert!(
            error.to_string().contains("journal sequence jumps"),
            "{error:#}"
        );
    }

    #[test]
    fn commit_prepared_refuses_sequence_exhaustion_at_u64_max() {
        let mut core = WorldCore::new("sequence-max", WorldLimits::default()).unwrap();
        core.sequence = u64::MAX;
        let event = WorldEvent {
            sequence: u64::MAX,
            participant_id: 0,
            kind: WorldEventKind::Chat("must not commit".to_owned()),
        };

        let error = core
            .commit_prepared(event)
            .expect_err("a saturated same-sequence event must be refused");

        assert!(error.to_string().contains("world sequence exhausted"));
        assert_eq!(core.sequence, u64::MAX);
        assert!(core.events.is_empty());
    }

    #[test]
    fn restore_accepts_v1_through_v6_headers_and_refuses_newer_at_the_header() {
        // The issue-#227 rollback contract: this binary reads versions
        // 1..=MAX, and anything newer is refused at the header check —
        // which is exactly where a version-1 binary refuses a version-2
        // journal, before any entry is applied.
        for version in [
            JOURNAL_VERSION,
            JOURNAL_VERSION_EXECUTION_PROFILE,
            JOURNAL_VERSION_REGION_HANDOFF,
            JOURNAL_VERSION_WORLD_BUDGET,
            JOURNAL_VERSION_OPEN_PRESENCE,
            JOURNAL_VERSION_CHECKPOINT_PARTS,
        ] {
            let restored = WorldCore::restore(
                "w",
                WorldLimits::default(),
                [JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version,
                }],
            )
            .unwrap();
            assert_eq!(restored.core.world_id, "w");
        }
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [JournalEntry::Created {
                world_id: "w".to_owned(),
                version: MAX_JOURNAL_VERSION + 1,
            }],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported world journal version"),
            "{error:#}"
        );
    }

    #[test]
    fn builder_installed_requires_v2_and_a_v1_reader_refuses_that_header() {
        let entry = JournalEntry::BuilderInstalled {
            by: 7,
            module_hash: "module".to_owned(),
            seed: 9,
        };
        assert_eq!(
            journal_version_required_by(&entry),
            JOURNAL_VERSION_EXECUTION_PROFILE
        );
        // This exercises the same header check as restore, with the maximum
        // version supported by a version-1 binary. It rejects before replaying
        // any entries from the journal.
        let error = crate::world::core::check_journal_version(
            JOURNAL_VERSION_EXECUTION_PROFILE,
            JOURNAL_VERSION,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported world journal version")
        );
    }

    fn handoff_event() -> WorldEvent {
        WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: "{\"handoff\":true}".to_owned(),
            },
        }
    }

    fn export_entry() -> JournalEntry {
        JournalEntry::EntityExported {
            handoff_id: "ab".repeat(16),
            entity_ref: "entity-1".to_owned(),
            source_region: "region-a".to_owned(),
            target_region: "region-b".to_owned(),
            prior_owner_epoch: 4,
            source_event: handoff_event(),
            payload_schema: "entity/v1".to_owned(),
            program_module_hash: "module".to_owned(),
            payload: vec![1, 2, 3],
            payload_digest: hex_encode(blake3::hash(&[1, 2, 3]).as_bytes()),
        }
    }

    fn export_transfer(handoff_id: &str) -> HandoffTransferState {
        HandoffTransferState {
            handoff_id: handoff_id.to_owned(),
            entity_ref: "entity-1".to_owned(),
            source_region: "region-a".to_owned(),
            target_region: "region-b".to_owned(),
            direction: HandoffDirection::Export,
            prior_owner_epoch: 4,
            new_owner_epoch: None,
            program_module_hash: "module".to_owned(),
            payload_schema: "entity/v1".to_owned(),
            payload_digest: "digest".to_owned(),
            payload: Some(vec![1, 2, 3]),
            event: handoff_event(),
        }
    }

    fn import_transfer(
        handoff_id: &str,
        prior_owner_epoch: u64,
        new_owner_epoch: u64,
    ) -> HandoffTransferState {
        HandoffTransferState {
            handoff_id: handoff_id.to_owned(),
            entity_ref: "entity-2".to_owned(),
            source_region: "region-b".to_owned(),
            target_region: "region-a".to_owned(),
            direction: HandoffDirection::Import,
            prior_owner_epoch,
            new_owner_epoch: Some(new_owner_epoch),
            program_module_hash: "module".to_owned(),
            payload_schema: "entity/v1".to_owned(),
            payload_digest: "digest-2".to_owned(),
            payload: None,
            event: handoff_event(),
        }
    }

    fn checkpoint_with_transfers(transfers: Vec<HandoffTransferState>) -> Vec<JournalEntry> {
        vec![
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION_REGION_HANDOFF,
            },
            JournalEntry::Checkpoint {
                sequence: 0,
                module_hash: "module".to_owned(),
                seed: 1,
                provenance: None,
                execution_profile: None,
                caps_ceiling: None,
                schedules: None,
                kv: None,
                blobs: None,
                transfers: Some(transfers),
                lease: None,
                snapshot: "state".to_owned(),
                events: Vec::new(),
                open_presence: None,
                admin_history: None,
            },
        ]
    }

    fn assert_checkpoint_transfer_refused(
        transfer_list: Vec<HandoffTransferState>,
        handoff_id: &str,
    ) -> String {
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            checkpoint_with_transfers(transfer_list),
        )
        .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(handoff_id), "{error}");
        error
    }

    #[test]
    fn handoff_entries_require_journal_version_3() {
        // #3870 phase 1: handoff kinds are the first entries a version-2
        // binary cannot read, so they pin the header to version 3 — the
        // same bump-on-first-write contract as the execution profile (#227).
        assert_eq!(
            journal_version_required_by(&export_entry()),
            JOURNAL_VERSION_REGION_HANDOFF
        );
        let entity_input = JournalEntry::Committed {
            event: WorldEvent {
                sequence: 1,
                participant_id: 7,
                kind: WorldEventKind::EntityInput {
                    entity_ref: "entity-1".to_owned(),
                    input: vec![9],
                },
            },
        };
        assert_eq!(
            journal_version_required_by(&entity_input),
            JOURNAL_VERSION_REGION_HANDOFF
        );
        let plain = JournalEntry::Committed {
            event: WorldEvent {
                sequence: 1,
                participant_id: 7,
                kind: WorldEventKind::Chat("hi".to_owned()),
            },
        };
        assert_eq!(journal_version_required_by(&plain), JOURNAL_VERSION);
        // A checkpoint whose retained events include an EntityInput event
        // requires version 3 even with no active handoff (PR #3922 Codex).
        let checkpoint = JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            admin_history: None,
            open_presence: None,
            snapshot: "s".to_owned(),
            events: vec![WorldEvent {
                sequence: 1,
                participant_id: 7,
                kind: WorldEventKind::EntityInput {
                    entity_ref: "entity-1".to_owned(),
                    input: vec![9],
                },
            }],
        };
        assert_eq!(
            journal_version_required_by(&checkpoint),
            JOURNAL_VERSION_REGION_HANDOFF
        );
        // Coordinator records fix their wire shape in this packet too.
        let started = CoordinatorJournalEntry::HandoffStarted {
            handoff_id: "ab".repeat(16),
            entity_ref: "entity-1".to_owned(),
            source_region: "region-a".to_owned(),
            target_region: "region-b".to_owned(),
            expected_owner_epoch: 4,
            region_map_epoch: 2,
            program_module_hash: "module".to_owned(),
            payload_schema: "entity/v1".to_owned(),
            requested_at: 42,
        };
        let json = serde_json::to_string(&started).unwrap();
        assert_eq!(
            serde_json::from_str::<CoordinatorJournalEntry>(&json).unwrap(),
            started
        );
    }

    #[test]
    fn handoff_transfers_survive_restore_and_checkpoint_compaction() {
        let entries = vec![
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION_REGION_HANDOFF,
            },
            export_entry(),
            JournalEntry::EntityImported {
                handoff_id: "cd".repeat(16),
                entity_ref: "entity-2".to_owned(),
                source_region: "region-b".to_owned(),
                target_region: "region-a".to_owned(),
                prior_owner_epoch: 1,
                new_owner_epoch: 2,
                program_module_hash: "module".to_owned(),
                payload_schema: "entity/v1".to_owned(),
                payload_digest: hex_encode(blake3::hash(&[1, 2, 3]).as_bytes()),
                destination_event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: serde_json::json!({
                            "protocol": "world.entity-transfer.v1",
                            "direction": "import",
                            "handoff_id": "cd".repeat(16),
                            "entity_ref": "entity-2",
                            "payload_digest": hex_encode(blake3::hash(&[1, 2, 3]).as_bytes()),
                            "payload_hex": "010203",
                        })
                        .to_string(),
                    },
                },
            },
        ];
        let restored = WorldCore::restore("w", WorldLimits::default(), entries).unwrap();
        let transfers = restored.core.handoff_transfers().to_vec();
        assert_eq!(transfers.len(), 2);
        assert_eq!(transfers[0].direction, HandoffDirection::Export);
        assert_eq!(transfers[0].payload.as_deref(), Some(&[1, 2, 3][..]));
        assert_eq!(transfers[1].direction, HandoffDirection::Import);
        assert_eq!(transfers[1].new_owner_epoch, Some(2));

        // Compaction must preserve active handoffs (fence, payload, receipt,
        // epochs). New checkpoints also carry known-empty presence metadata,
        // so their header uses version 5, above the handoff version 3.
        let mut core = restored.core;
        core.active_module_hash = Some("h".to_owned());
        let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(matches!(
            compacted[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_OPEN_PRESENCE,
                ..
            }
        ));
        let restored = WorldCore::restore("w", WorldLimits::default(), compacted).unwrap();
        assert_eq!(restored.core.handoff_transfers(), transfers.as_slice());
    }

    fn lease_header() -> JournalEntry {
        // Stamped lease entries (#13711) require version 16; the
        // unstamped legacy header (version 7) is covered by
        // `unstamped_lease_entries_still_restore_at_version_7`.
        JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_LEASE_POSITION,
        }
    }

    fn spill_handoff() -> JournalEntry {
        JournalEntry::LeaseHandedOff {
            from: DO_WRITE_LEASE_HOLDER.to_owned(),
            to: "native-1".to_owned(),
            epoch: 1,
            expires_at_ms: Some(30_000),
            position: 1,
        }
    }

    fn spill_renewal() -> JournalEntry {
        JournalEntry::LeaseRenewed {
            holder: "native-1".to_owned(),
            epoch: 1,
            expires_at_ms: 40_000,
            position: 2,
        }
    }

    fn stale_rejection() -> JournalEntry {
        // The native holder presenting its pre-handoff epoch: the gate
        // refuses it as stale, and the authority journals the attempt.
        JournalEntry::LeaseWriteRejected {
            writer: "native-1".to_owned(),
            presented_epoch: 0,
            operation_id: "op-1".to_owned(),
            observed_epoch: 1,
            reason: LeaseRejectionReason::StaleEpoch,
            received_at_ms: 35_000,
            position: 3,
        }
    }

    #[test]
    fn write_lease_entries_require_journal_version_7() {
        // Design #2819 phase 2 (#3804): lease kinds are the first
        // entries a version-6 binary cannot read, so they pin the
        // header to version 7 under the #227 bump-on-first-write
        // contract. Unstamped (position 0) entries still require
        // exactly 7; a stamped position (#13711) requires 16.
        for entry in [
            JournalEntry::LeaseGranted {
                holder: "native-1".to_owned(),
                epoch: 1,
                expires_at_ms: Some(30_000),
                position: 0,
            },
            JournalEntry::LeaseRenewed {
                holder: "native-1".to_owned(),
                epoch: 1,
                expires_at_ms: 40_000,
                position: 0,
            },
            JournalEntry::LeaseHandedOff {
                from: DO_WRITE_LEASE_HOLDER.to_owned(),
                to: "native-1".to_owned(),
                epoch: 1,
                expires_at_ms: Some(30_000),
                position: 0,
            },
            JournalEntry::LeaseRevoked {
                former_holder: "native-1".to_owned(),
                epoch: 2,
                reason: "expired".to_owned(),
                position: 0,
            },
            JournalEntry::LeaseWriteRejected {
                writer: "native-1".to_owned(),
                presented_epoch: 0,
                operation_id: "op-1".to_owned(),
                observed_epoch: 1,
                reason: LeaseRejectionReason::StaleEpoch,
                received_at_ms: 35_000,
                position: 0,
            },
        ] {
            assert_eq!(
                journal_version_required_by(&entry),
                JOURNAL_VERSION_WRITE_LEASE,
                "{entry:?}"
            );
        }
        for entry in [spill_handoff(), spill_renewal(), stale_rejection()] {
            assert_eq!(
                journal_version_required_by(&entry),
                JOURNAL_VERSION_LEASE_POSITION,
                "{entry:?}"
            );
        }
        // The max-version pin moved to version 11 with program-bundle
        // installs (#8280), after version 10 for portal grants (F12.14
        // wiring, code#1068), version 9 for signed play (#4346) and
        // version 8 for recurring schedules (#1355); the lease entries
        // still require exactly 7.
        // Version 12 (#11960) records join and revoke times, version
        // 13 the kv store entries, version 14 the blob store entries,
        // and version 15 the notify provider's `Notification` events
        // (all #1062). Version 16 (#13711) stamps lease positions.
        assert_eq!(MAX_JOURNAL_VERSION, JOURNAL_VERSION_LEASE_POSITION);
        // A checkpoint carrying a non-bootstrap lease also requires
        // version 7; one without a lease keeps its existing version.
        // A checkpoint whose lease history carries stamped positions
        // requires version 16.
        let mut lease = WriteLease::bootstrap();
        lease
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "native-1", 1, Some(30_000), 0)
            .unwrap();
        let mut stamped = WriteLease::bootstrap();
        stamped
            .apply_handoff(DO_WRITE_LEASE_HOLDER, "native-1", 1, Some(30_000), 1)
            .unwrap();
        let checkpoint = |lease: Option<WriteLease>| JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease,
            snapshot: "s".to_owned(),
            events: Vec::new(),
            open_presence: None,
            admin_history: None,
        };
        assert_eq!(
            journal_version_required_by(&checkpoint(Some(lease))),
            JOURNAL_VERSION_WRITE_LEASE
        );
        assert_eq!(
            journal_version_required_by(&checkpoint(Some(stamped))),
            JOURNAL_VERSION_LEASE_POSITION
        );
        assert_eq!(
            journal_version_required_by(&checkpoint(None)),
            JOURNAL_VERSION
        );
    }

    #[test]
    fn write_lease_wire_shape_round_trips() {
        for entry in [spill_handoff(), spill_renewal(), stale_rejection()] {
            let json = serde_json::to_string(&entry).unwrap();
            assert_eq!(
                serde_json::from_str::<JournalEntry>(&json).unwrap(),
                entry,
                "{json}"
            );
        }
        // A lease entry in a journal whose header predates the
        // version it requires is refused at restore, never applied
        // mid-read. The handoff helper carries a stamped position
        // (#13711), so it requires version 16.
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION_CHECKPOINT_PARTS,
                },
                spill_handoff(),
            ],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("requires version 16"),
            "{error:#}"
        );
    }

    #[test]
    fn write_lease_replay_walks_grant_renew_revoke_handoff() {
        // Bootstrap: an untouched world is DO-owned at epoch 0.
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION,
            }],
        )
        .unwrap();
        assert_eq!(restored.core.write_lease(), &WriteLease::bootstrap());
        assert_eq!(
            restored
                .core
                .write_lease()
                .check_writer(DO_WRITE_LEASE_HOLDER, 0, u64::MAX),
            Ok(())
        );

        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                spill_renewal(),
                JournalEntry::LeaseRevoked {
                    former_holder: "native-1".to_owned(),
                    epoch: 2,
                    reason: "term expired without renewal".to_owned(),
                    position: 3,
                },
                JournalEntry::LeaseGranted {
                    holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                    epoch: 3,
                    expires_at_ms: None,
                    position: 4,
                },
            ],
        )
        .unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.holder.as_deref(), Some(DO_WRITE_LEASE_HOLDER));
        assert_eq!(lease.epoch, 3);
        assert_eq!(lease.expires_at_ms, None);
        let kinds: Vec<LeaseTransitionKind> = lease
            .transitions()
            .map(|transition| transition.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                LeaseTransitionKind::HandedOff,
                LeaseTransitionKind::Renewed,
                LeaseTransitionKind::Revoked,
                LeaseTransitionKind::Granted,
            ]
        );
        // Positions ride the history in journal order (#13711), and
        // the watermark is the highest position replayed.
        let positions: Vec<u64> = lease
            .transitions()
            .map(|transition| transition.position)
            .collect();
        assert_eq!(positions, [1, 2, 3, 4]);
        assert_eq!(restored.core.lease_position_watermark(), 4);
        // The ex-holder is fenced at the new epoch, and the returned
        // holder presenting an old epoch is stale.
        assert_eq!(
            lease.check_writer("native-1", 1, 0),
            Err(LeaseRejectionReason::NotHolder)
        );
        assert_eq!(
            lease.check_writer(DO_WRITE_LEASE_HOLDER, 2, 0),
            Err(LeaseRejectionReason::StaleEpoch)
        );
        assert_eq!(
            lease.check_writer(DO_WRITE_LEASE_HOLDER, 3, u64::MAX),
            Ok(())
        );
    }

    #[test]
    fn write_lease_gate_fences_an_expired_holder_before_any_revoke() {
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [lease_header(), spill_handoff(), spill_renewal()],
        )
        .unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.check_writer("native-1", 1, 39_999), Ok(()));
        // Expiry alone fences the holder: no revocation entry needs to
        // land first, because the gate judges the term at append time.
        assert_eq!(
            lease.check_writer("native-1", 1, 40_000),
            Err(LeaseRejectionReason::Expired)
        );
        assert!(lease.is_expired(40_000));
        assert!(!lease.is_expired(39_999));
    }

    #[test]
    fn write_lease_rejected_attempt_is_audited_and_never_applied() {
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [lease_header(), spill_handoff(), stale_rejection()],
        )
        .unwrap();
        let lease = restored.core.write_lease();
        // The audit landed; the attempted write did not change the
        // holder, the epoch, or any program state.
        assert_eq!(lease.holder.as_deref(), Some("native-1"));
        assert_eq!(lease.epoch, 1);
        assert_eq!(lease.rejections().count(), 1);
        assert_eq!(lease.rejections().next().unwrap().operation_id, "op-1");
        assert_eq!(restored.core.current_sequence(), 0);
        assert!(restored.replay.is_empty());

        // A second audit of the same attempt is a duplicate.
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                stale_rejection(),
                stale_rejection(),
            ],
        )
        .unwrap_err();
        assert!(error.to_string().contains("twice"), "{error:#}");

        // An audit whose stated reason disagrees with the gate is
        // refused: at 35s the native term (40s after renewal) had not
        // expired, so this attempt was stale, not expired.
        let lying = JournalEntry::LeaseWriteRejected {
            writer: "native-1".to_owned(),
            presented_epoch: 0,
            operation_id: "op-1".to_owned(),
            observed_epoch: 1,
            reason: LeaseRejectionReason::Expired,
            received_at_ms: 35_000,
            position: 2,
        };
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [lease_header(), spill_handoff(), lying],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("disagrees with the lease gate"),
            "{error:#}"
        );
    }

    #[test]
    fn write_lease_transitions_refuse_epoch_reuse_skips_and_non_holders() {
        let cases: Vec<(Vec<JournalEntry>, &str)> = vec![
            (
                // A handoff that skips epoch 1.
                vec![JournalEntry::LeaseHandedOff {
                    from: DO_WRITE_LEASE_HOLDER.to_owned(),
                    to: "native-1".to_owned(),
                    epoch: 2,
                    expires_at_ms: Some(30_000),
                    position: 1,
                }],
                "advances the write lease epoch from 0 to 2",
            ),
            (
                // A grant while the DO still holds the lease.
                vec![JournalEntry::LeaseGranted {
                    holder: "native-1".to_owned(),
                    epoch: 1,
                    expires_at_ms: None,
                    position: 1,
                }],
                "while",
            ),
            (
                // A renewal by someone who does not hold the lease.
                vec![
                    spill_handoff(),
                    JournalEntry::LeaseRenewed {
                        holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                        epoch: 1,
                        expires_at_ms: 50_000,
                        position: 2,
                    },
                ],
                "held by",
            ),
            (
                // A renewal that moves the expiry backwards.
                vec![
                    spill_handoff(),
                    spill_renewal(),
                    JournalEntry::LeaseRenewed {
                        holder: "native-1".to_owned(),
                        epoch: 1,
                        expires_at_ms: 35_000,
                        position: 3,
                    },
                ],
                "backwards",
            ),
            (
                // A revocation naming the wrong former holder.
                vec![
                    spill_handoff(),
                    JournalEntry::LeaseRevoked {
                        former_holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                        epoch: 2,
                        reason: "operator".to_owned(),
                        position: 2,
                    },
                ],
                "held by",
            ),
            (
                // A handoff from someone who does not hold the lease.
                vec![JournalEntry::LeaseHandedOff {
                    from: "native-9".to_owned(),
                    to: "native-1".to_owned(),
                    epoch: 1,
                    expires_at_ms: None,
                    position: 1,
                }],
                "held by",
            ),
        ];
        for (entries, needle) in cases {
            let mut journal = vec![lease_header()];
            journal.extend(entries);
            let error = WorldCore::restore("w", WorldLimits::default(), journal).unwrap_err();
            assert!(error.to_string().contains(needle), "{error:#}");
        }
    }

    #[test]
    fn write_lease_rejection_audit_stays_valid_across_later_transitions() {
        // Codex P1 on PR #5571: an audit is valid against the lease
        // state at its own journal position, not the final state.
        // Handoff (epoch 1) -> audit a stale write (observed epoch 1)
        // -> revoke (epoch 2) -> grant (epoch 3) must restore from the
        // original journal AND from its compaction: a validator that
        // replays every transition before any audit would refuse the
        // audit for naming observed epoch 1 against a final epoch of
        // 3, bricking the world after compaction.
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                stale_rejection(),
                JournalEntry::LeaseRevoked {
                    former_holder: "native-1".to_owned(),
                    epoch: 2,
                    reason: "term expired without renewal".to_owned(),
                    position: 4,
                },
                JournalEntry::LeaseGranted {
                    holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                    epoch: 3,
                    expires_at_ms: None,
                    position: 5,
                },
            ],
        )
        .unwrap();
        let lease = restored.core.write_lease().clone();
        assert_eq!(lease.epoch, 3);
        assert_eq!(lease.rejections().count(), 1);
        lease.validate_history().unwrap();

        let mut core = restored.core;
        core.active_module_hash = Some("h".to_owned());
        let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
        let restored = WorldCore::restore("w", WorldLimits::default(), compacted).unwrap();
        assert_eq!(restored.core.write_lease(), &lease);
    }

    #[test]
    fn write_lease_survives_restore_and_checkpoint_compaction() {
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                spill_renewal(),
                stale_rejection(),
            ],
        )
        .unwrap();
        let lease = restored.core.write_lease().clone();

        // Compaction carries the lease whole on the checkpoint: the
        // header rises to version 16 (stamped positions, #13711) and
        // a restore of the compacted journal rebuilds the identical
        // lease — epoch, holder, term, transition history and
        // rejection audit all preserved.
        let mut core = restored.core;
        core.active_module_hash = Some("h".to_owned());
        let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(matches!(
            compacted[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_LEASE_POSITION,
                ..
            }
        ));
        let checkpoint_lease = compacted.iter().find_map(|entry| match entry {
            JournalEntry::Checkpoint { lease, .. } => lease.clone(),
            _ => None,
        });
        assert_eq!(checkpoint_lease, Some(lease.clone()));
        let restored = WorldCore::restore("w", WorldLimits::default(), compacted.clone()).unwrap();
        assert_eq!(restored.core.write_lease(), &lease);

        // A checkpoint whose carried history does not reproduce its
        // claimed state (here: the epoch was bumped without a matching
        // transition) is refused whole.
        let mut tampered = compacted;
        for entry in &mut tampered {
            if let JournalEntry::Checkpoint {
                lease: Some(lease), ..
            } = entry
            {
                lease.epoch += 1;
            }
        }
        let error = WorldCore::restore("w", WorldLimits::default(), tampered).unwrap_err();
        assert!(error.to_string().contains("write lease"), "{error:#}");
    }

    #[test]
    fn lease_position_watermark_reports_the_grant_position_after_replay() {
        // #13711 oracle: a successor replaying a journal with a
        // grant at position P reports watermark >= P before it may
        // accept writes. Bootstrap has no stamped position.
        let bootstrap = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION,
            }],
        )
        .unwrap();
        assert_eq!(bootstrap.core.lease_position_watermark(), 0);

        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                JournalEntry::LeaseRevoked {
                    former_holder: "native-1".to_owned(),
                    epoch: 2,
                    reason: "term expired without renewal".to_owned(),
                    position: 2,
                },
                JournalEntry::LeaseGranted {
                    holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                    epoch: 3,
                    expires_at_ms: None,
                    position: 3,
                },
            ],
        )
        .unwrap();
        assert_eq!(restored.core.lease_position_watermark(), 3);
        assert_eq!(restored.core.write_lease().last_position(), 3);

        // Compaction preserves the watermark: the checkpoint carries
        // the whole lease history, positions included.
        let mut core = restored.core;
        core.active_module_hash = Some("h".to_owned());
        let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
        let restored = WorldCore::restore("w", WorldLimits::default(), compacted).unwrap();
        assert_eq!(restored.core.lease_position_watermark(), 3);
    }

    #[test]
    fn unstamped_lease_entries_still_restore_at_version_7() {
        // #13711 back-compat: lease entries written before positions
        // existed parse with position 0, restore under a version-7
        // header, and report watermark 0 — the barrier simply has
        // no stamped grant to name yet.
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION_WRITE_LEASE,
                },
                JournalEntry::LeaseHandedOff {
                    from: DO_WRITE_LEASE_HOLDER.to_owned(),
                    to: "native-1".to_owned(),
                    epoch: 1,
                    expires_at_ms: Some(30_000),
                    position: 0,
                },
                JournalEntry::LeaseRenewed {
                    holder: "native-1".to_owned(),
                    epoch: 1,
                    expires_at_ms: 40_000,
                    position: 0,
                },
            ],
        )
        .unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.holder.as_deref(), Some("native-1"));
        assert_eq!(lease.epoch, 1);
        assert_eq!(restored.core.lease_position_watermark(), 0);
        assert!(
            lease
                .transitions()
                .all(|transition| transition.position == 0)
        );
    }

    #[test]
    fn lease_positions_that_do_not_advance_are_refused() {
        // #13711: a stamped position that does not advance past the
        // highest stamped position already replayed is a corrupt
        // journal, refused at restore — not silently reordered.
        let error = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                lease_header(),
                spill_handoff(),
                JournalEntry::LeaseRenewed {
                    holder: "native-1".to_owned(),
                    epoch: 1,
                    expires_at_ms: 40_000,
                    position: 1,
                },
            ],
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("does not advance past"),
            "{error:#}"
        );
    }

    #[test]
    fn lease_positions_are_strictly_increasing_across_appends_and_compaction() {
        // #13711 property test. Oracle: the lease history after
        // restore (and after compaction + restore) carries exactly
        // the positions the appends stamped, in order — one per
        // lease record, strictly increasing. The generator folds
        // op bytes through the lease state machine (holder, epoch,
        // expiry), so every sequence is one the append authority
        // could legally have written: handoff/renew/revoke while a
        // holder stands, grant only while none does.
        use proptest::prelude::*;
        proptest!(|(ops in proptest::collection::vec(0u8..=255, 1..=12))| {
            #[derive(Clone, Copy, PartialEq, Eq)]
            enum ModelHolder {
                Do,
                Native,
            }
            let mut holder = Some(ModelHolder::Do);
            let mut epoch = 0u64;
            let mut expiry: Option<u64> = None;
            let mut entries = vec![lease_header()];
            let mut position = 0u64;
            for op in ops {
                position += 1;
                match holder {
                    Some(current) => match op % 3 {
                        0 => {
                            let (from, to) = match current {
                                ModelHolder::Do => (DO_WRITE_LEASE_HOLDER, "native-1"),
                                ModelHolder::Native => ("native-1", DO_WRITE_LEASE_HOLDER),
                            };
                            epoch += 1;
                            let next_expiry = if current == ModelHolder::Do {
                                Some(30_000 + epoch * 1_000)
                            } else {
                                None
                            };
                            entries.push(JournalEntry::LeaseHandedOff {
                                from: from.to_owned(),
                                to: to.to_owned(),
                                epoch,
                                expires_at_ms: next_expiry,
                                position,
                            });
                            holder = Some(if current == ModelHolder::Do {
                                ModelHolder::Native
                            } else {
                                ModelHolder::Do
                            });
                            expiry = next_expiry;
                        }
                        1 => {
                            let next_expiry = expiry.map_or(40_000, |e| e + 1_000);
                            let holder_name = match current {
                                ModelHolder::Do => DO_WRITE_LEASE_HOLDER,
                                ModelHolder::Native => "native-1",
                            };
                            entries.push(JournalEntry::LeaseRenewed {
                                holder: holder_name.to_owned(),
                                epoch,
                                expires_at_ms: next_expiry,
                                position,
                            });
                            expiry = Some(next_expiry);
                        }
                        _ => {
                            let former = match current {
                                ModelHolder::Do => DO_WRITE_LEASE_HOLDER,
                                ModelHolder::Native => "native-1",
                            };
                            epoch += 1;
                            entries.push(JournalEntry::LeaseRevoked {
                                former_holder: former.to_owned(),
                                epoch,
                                reason: "term expired without renewal".to_owned(),
                                position,
                            });
                            holder = None;
                            expiry = None;
                        }
                    },
                    None => {
                        epoch += 1;
                        if op % 2 == 0 {
                            entries.push(JournalEntry::LeaseGranted {
                                holder: DO_WRITE_LEASE_HOLDER.to_owned(),
                                epoch,
                                expires_at_ms: None,
                                position,
                            });
                            holder = Some(ModelHolder::Do);
                            expiry = None;
                        } else {
                            entries.push(JournalEntry::LeaseGranted {
                                holder: "native-1".to_owned(),
                                epoch,
                                expires_at_ms: Some(30_000 + epoch * 1_000),
                                position,
                            });
                            holder = Some(ModelHolder::Native);
                            expiry = Some(30_000 + epoch * 1_000);
                        }
                    }
                }
            }
            let expected: Vec<u64> = (1..=position).collect();
            let restored = WorldCore::restore("w", WorldLimits::default(), entries).unwrap();
            let positions: Vec<u64> = restored
                .core
                .write_lease()
                .transitions()
                .map(|transition| transition.position)
                .collect();
            prop_assert_eq!(positions, expected.clone());
            prop_assert_eq!(restored.core.lease_position_watermark(), position);

            let mut core = restored.core;
            core.active_module_hash = Some("h".to_owned());
            let compacted = core.checkpoint_entries("state".to_owned()).unwrap();
            let restored = WorldCore::restore("w", WorldLimits::default(), compacted).unwrap();
            let positions: Vec<u64> = restored
                .core
                .write_lease()
                .transitions()
                .map(|transition| transition.position)
                .collect();
            prop_assert_eq!(positions, expected);
            prop_assert_eq!(restored.core.lease_position_watermark(), position);
        });
    }

    #[test]
    fn checkpoint_handoff_transfers_validate_duplicates_epochs_and_events() {
        for (first, second) in [
            (
                export_transfer("duplicate-export"),
                export_transfer("duplicate-export"),
            ),
            (
                import_transfer("duplicate-import", 4, 5),
                import_transfer("duplicate-import", 4, 5),
            ),
        ] {
            let handoff_id = first.handoff_id.clone();
            let error = assert_checkpoint_transfer_refused(vec![first, second], &handoff_id);
            assert!(error.contains("twice"), "{error}");
        }

        let invalid_import = import_transfer("bad-epoch", 5, 5);
        let error = assert_checkpoint_transfer_refused(vec![invalid_import], "bad-epoch");
        assert!(
            error.contains("does not advance the owner epoch"),
            "{error}"
        );

        let mut import_without_epoch = import_transfer("missing-epoch", 4, 5);
        import_without_epoch.new_owner_epoch = None;
        let error = assert_checkpoint_transfer_refused(vec![import_without_epoch], "missing-epoch");
        assert!(
            error.contains("does not advance the owner epoch"),
            "{error}"
        );

        let mut invalid_export_epoch = export_transfer("export-epoch");
        invalid_export_epoch.new_owner_epoch = Some(5);
        let error = assert_checkpoint_transfer_refused(vec![invalid_export_epoch], "export-epoch");
        assert!(error.contains("carries a new owner epoch"), "{error}");

        for (mut invalid_event, sequence, participant_id) in [
            (export_transfer("bad-export-event"), 1, 0),
            (import_transfer("bad-import-event", 4, 5), 0, 7),
        ] {
            invalid_event.event.sequence = sequence;
            invalid_event.event.participant_id = participant_id;
            let handoff_id = invalid_event.handoff_id.clone();
            let error = assert_checkpoint_transfer_refused(vec![invalid_event], &handoff_id);
            assert!(error.contains("event claims a sequence"), "{error}");
        }
    }

    #[test]
    fn checkpoint_accepts_valid_export_and_import_transfers() {
        let transfers = vec![
            export_transfer("valid-export"),
            import_transfer("valid-import", 4, 5),
        ];
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            checkpoint_with_transfers(transfers.clone()),
        )
        .unwrap();
        assert_eq!(restored.core.handoff_transfers(), transfers.as_slice());
    }

    #[test]
    fn handoff_import_at_max_owner_epoch_is_refused() {
        // #3961: prior = u64::MAX must never validate an import — the plain
        // `prior + 1` wrapped to 0 in release (accepting new = 0) and panicked
        // in debug/test. The checked form refuses every new epoch.
        for new_owner_epoch in [0, 1, u64::MAX] {
            let entries = vec![
                JournalEntry::Created {
                    world_id: "w".to_owned(),
                    version: JOURNAL_VERSION_REGION_HANDOFF,
                },
                JournalEntry::EntityImported {
                    handoff_id: "cd".repeat(16),
                    entity_ref: "entity-2".to_owned(),
                    source_region: "region-b".to_owned(),
                    target_region: "region-a".to_owned(),
                    prior_owner_epoch: u64::MAX,
                    new_owner_epoch,
                    program_module_hash: "module".to_owned(),
                    payload_schema: "entity/v1".to_owned(),
                    payload_digest: "digest-2".to_owned(),
                    destination_event: handoff_event(),
                },
            ];
            let error = WorldCore::restore("w", WorldLimits::default(), entries).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("does not advance the owner epoch"),
                "new={new_owner_epoch}: {error:#}"
            );
        }
    }

    #[test]
    fn duplicate_handoff_export_in_journal_is_refused() {
        let entries = vec![
            JournalEntry::Created {
                world_id: "w".to_owned(),
                version: JOURNAL_VERSION_REGION_HANDOFF,
            },
            export_entry(),
            export_entry(),
        ];
        let error = WorldCore::restore("w", WorldLimits::default(), entries).unwrap_err();
        assert!(error.to_string().contains("twice"), "{error:#}");
    }

    #[test]
    fn execution_profile_survives_checkpoint_compaction_and_restore() {
        let profile = crate::world_limits::WorldExecutionProfile {
            wasm_fuel: Some(1234),
            native_cpu_seconds: Some(3),
            ..Default::default()
        };
        let mut core = world(8);
        core.execution_profile = profile;
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        // The checkpoint re-emits the profile and known-empty open-presence
        // metadata, so its journal header must carry the newest required
        // format version (#227).
        assert!(matches!(
            entries[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_OPEN_PRESENCE,
                ..
            }
        ));
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.execution_profile(), profile);
    }

    /// The total logical charge `WorldCore::restore` accounts for a
    /// journal: the fixed core charge plus each entry's serialized
    /// length and structural allowance (#557 Phase 2).
    fn journal_charge(entries: &[JournalEntry]) -> u64 {
        let mut total = crate::world_limits::WORLD_MEMORY_BASE_BYTES;
        for entry in entries {
            let len = serde_json::to_vec(entry).unwrap().len();
            total += crate::world_limits::journal_entry_charge(len);
        }
        total
    }

    #[test]
    fn restore_charges_the_journal_against_the_default_budget() {
        // #557 Phase 2: a budgetless journal restores under the default
        // ceiling, and the restored core reports the measured charge.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let expected_charge = journal_charge(&entries);
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        let budget = restored.core.memory_budget();
        assert_eq!(
            budget.limit_bytes(),
            crate::world_limits::DEFAULT_WORLD_TOTAL_MEMORY_BYTES
        );
        assert_eq!(budget.charged_bytes(), expected_charge);
        assert_eq!(budget.high_water_bytes(), expected_charge);
        assert_eq!(budget.refusal_count(), 0);
    }

    #[test]
    fn restore_refuses_a_journal_over_its_persisted_budget_and_retries() {
        // #557 Phase 2: the persisted profile budget is enforced at
        // restore with a typed refusal; refusal applies nothing, and
        // the same journal restores once its budget is raised.
        let profile = crate::world_limits::WorldExecutionProfile {
            total_memory_bytes: Some(1),
            ..Default::default()
        };
        let mut core = world(8);
        core.execution_profile = profile;
        core.active_module_hash = Some("h".to_owned());
        let mut entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let error =
            WorldCore::restore("test-world", WorldLimits::default(), entries.clone()).unwrap_err();
        let refusal = error
            .downcast_ref::<crate::world_limits::WorldMemoryBudgetRefusal>()
            .expect("an over-budget restore must fail with the typed budget refusal");
        assert_eq!(refusal.limit_bytes, 1);
        // Raise the persisted budget past the measured charge (with
        // headroom for the larger number's own serialized width): the
        // same entries now restore, charged exactly as accounted.
        let raised = journal_charge(&entries) + 1024;
        for entry in &mut entries {
            if let JournalEntry::Checkpoint {
                execution_profile, ..
            } = entry
            {
                execution_profile.as_mut().unwrap().total_memory_bytes = Some(raised);
            }
        }
        let expected = journal_charge(&entries);
        assert!(expected <= raised);
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.memory_budget().limit_bytes(), raised);
        assert_eq!(restored.core.memory_budget().charged_bytes(), expected);
    }

    #[test]
    fn restore_resolves_the_persisted_budget_against_the_current_server_ceiling() {
        // Codex P1 on PR #4628: restore must enforce the CURRENT server
        // ceiling, not the persisted request or the default. A journal
        // persisting an unclamped (over-ceiling) request is charged
        // against the lowered ceiling and refused with that limit.
        let profile = crate::world_limits::WorldExecutionProfile {
            total_memory_bytes: Some(u64::MAX),
            ..Default::default()
        };
        let mut core = world(8);
        core.execution_profile = profile;
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let lowered = journal_charge(&entries) - 1;
        let runtime = crate::world_limits::RuntimeLimits {
            total_memory_bytes: lowered,
            ..Default::default()
        };
        let error = WorldCore::restore_with_runtime(
            "test-world",
            WorldLimits::default(),
            runtime,
            entries.clone(),
        )
        .unwrap_err();
        let refusal = error
            .downcast_ref::<crate::world_limits::WorldMemoryBudgetRefusal>()
            .expect("restore over the current ceiling must fail with the typed refusal");
        assert_eq!(refusal.limit_bytes, lowered);
        // A budgetless journal inherits the CURRENT ceiling: raising it
        // reopens a journal the default ceiling would refuse, and the
        // restored budget reports the raised limit.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let raised = journal_charge(&entries) + 1024;
        let runtime = crate::world_limits::RuntimeLimits {
            total_memory_bytes: raised,
            ..Default::default()
        };
        let restored =
            WorldCore::restore_with_runtime("test-world", WorldLimits::default(), runtime, entries)
                .unwrap();
        assert_eq!(restored.core.memory_budget().limit_bytes(), raised);
    }

    #[test]
    fn restore_tracks_the_final_profile_including_a_budget_reset() {
        // Codex P2 on PR #4628: replay replaces the whole execution
        // profile, so a later profile with an unset budget resets an
        // earlier persisted budget instead of leaving it enforced.
        let profile = crate::world_limits::WorldExecutionProfile {
            total_memory_bytes: Some(1),
            ..Default::default()
        };
        let mut core = world(8);
        core.execution_profile = profile;
        core.active_module_hash = Some("h".to_owned());
        let mut entries = core.checkpoint_entries("state".to_owned()).unwrap();
        // Sanity: the persisted Some(1) budget alone refuses restore.
        assert!(WorldCore::restore("test-world", WorldLimits::default(), entries.clone()).is_err());
        entries.push(JournalEntry::ExecutionProfile {
            profile: crate::world_limits::WorldExecutionProfile::default(),
        });
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.core.memory_budget().limit_bytes(),
            crate::world_limits::DEFAULT_WORLD_TOTAL_MEMORY_BYTES
        );
        assert_eq!(restored.core.execution_profile().total_memory_bytes, None);
    }

    #[test]
    fn budget_entries_require_journal_version_4_while_budgetless_ones_do_not() {
        // #557 Phase 1: a persisted total budget is the first fact a
        // version-3 binary would parse yet silently ignore, so it pins the
        // header to version 4. Profiles without a budget keep version 2.
        let plain_profile = JournalEntry::ExecutionProfile {
            profile: crate::world_limits::WorldExecutionProfile {
                wasm_fuel: Some(1234),
                ..Default::default()
            },
        };
        assert_eq!(
            journal_version_required_by(&plain_profile),
            JOURNAL_VERSION_EXECUTION_PROFILE
        );
        let budget_profile = JournalEntry::ExecutionProfile {
            profile: crate::world_limits::WorldExecutionProfile {
                total_memory_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            },
        };
        assert_eq!(
            journal_version_required_by(&budget_profile),
            JOURNAL_VERSION_WORLD_BUDGET
        );
        // A budget checkpoint requires version 4 even with no handoff
        // content; the handoff version never outranks the budget version.
        let budget_checkpoint = JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: Some(crate::world_limits::WorldExecutionProfile {
                total_memory_bytes: Some(32 * 1024 * 1024),
                ..Default::default()
            }),
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: vec![],
            open_presence: None,
            admin_history: None,
        };
        assert_eq!(
            journal_version_required_by(&budget_checkpoint),
            JOURNAL_VERSION_WORLD_BUDGET
        );
        let plain_checkpoint = JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: vec![],
            open_presence: None,
            admin_history: None,
        };
        assert_eq!(
            journal_version_required_by(&plain_checkpoint),
            JOURNAL_VERSION
        );

        for part in [
            JournalEntry::CheckpointSnapshotPart {
                sequence: 1,
                part_index: 0,
                part_count: 1,
                bytes: "state".to_owned(),
            },
            JournalEntry::CheckpointEventsPart {
                sequence: 1,
                part_index: 0,
                part_count: 1,
                events: Vec::new(),
            },
            JournalEntry::CheckpointAdminPart {
                sequence: 1,
                part_index: 0,
                part_count: 1,
                events: Vec::new(),
            },
        ] {
            assert_eq!(
                journal_version_required_by(&part),
                JOURNAL_VERSION_CHECKPOINT_PARTS
            );
        }
    }

    #[test]
    fn budget_checkpoint_compacts_to_version_6_and_restores() {
        let profile = crate::world_limits::WorldExecutionProfile {
            total_memory_bytes: Some(32 * 1024 * 1024),
            ..Default::default()
        };
        let mut core = world(8);
        core.execution_profile = profile;
        core.active_module_hash = Some("h".to_owned());
        // An oversized snapshot forces the bounded part entries, so the
        // header is version 6, above the open-presence and budget
        // requirements (#227).
        let entries = core.checkpoint_entries("state".repeat(50_000)).unwrap();
        assert!(matches!(
            entries[0],
            JournalEntry::Created {
                version: JOURNAL_VERSION_CHECKPOINT_PARTS,
                ..
            }
        ));
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.execution_profile(), profile);
    }

    #[test]
    fn checkpoint_parts_are_utf8_safe_bounded_and_restore_all_state() {
        let mut core = world(16);
        core.active_module_hash = Some("h".to_owned());
        core.sequence = 3;
        let (participant, _) = join(&mut core, "ann");
        core.open_presence.insert(participant.id);

        let events: Vec<WorldEvent> = (1..=3)
            .map(|sequence| WorldEvent {
                sequence,
                participant_id: participant.id,
                kind: WorldEventKind::Chat(format!("{}-{sequence}", "é\"".repeat(35_000))),
            })
            .collect();
        core.events = events.clone().into();
        core.admin_history = std::iter::once(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(participant.id, "ann", true),
            },
        })
        .chain(events.iter().cloned())
        .collect();
        core.admin_history_complete = true;
        let snapshot = "🧭\"".repeat(80_000);

        let entries = core.checkpoint_entries(snapshot.clone()).unwrap();
        assert!(matches!(
            entries.first(),
            Some(JournalEntry::Created { version, .. }) if *version >= JOURNAL_VERSION_CHECKPOINT_PARTS
        ));
        assert!(entries.iter().all(|entry| {
            serde_json::to_vec(entry).is_ok_and(|bytes| bytes.len() <= MAX_CHECKPOINT_PART_BYTES)
        }));

        let snapshot_parts: Vec<&str> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::CheckpointSnapshotPart { bytes, .. } => Some(bytes.as_str()),
                _ => None,
            })
            .collect();
        assert!(snapshot_parts.len() > 1);
        assert_eq!(snapshot_parts.concat(), snapshot);
        let event_parts: Vec<&Vec<WorldEvent>> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::CheckpointEventsPart { events, .. } => Some(events),
                _ => None,
            })
            .collect();
        assert!(event_parts.len() > 1);
        assert_eq!(
            event_parts
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            events
        );
        let admin_parts: Vec<&Vec<WorldEvent>> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::CheckpointAdminPart { events, .. } => Some(events),
                _ => None,
            })
            .collect();
        assert!(admin_parts.len() > 1);
        let expected_admin: Vec<WorldEvent> = std::iter::once(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(participant.id, "ann", true),
            },
        })
        .chain(events.iter().cloned())
        .collect();
        assert_eq!(
            admin_parts
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            expected_admin
        );
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::Checkpoint {
                snapshot,
                events,
                admin_history: Some(JournalAdminHistory { events: admin_events, complete: true }),
                open_presence: Some(presence),
                ..
            }) if snapshot.is_empty()
                && events.is_empty()
                && admin_events.is_empty()
                && presence == &vec![participant.id]
        ));

        let restored =
            WorldCore::restore_replica("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.program.unwrap().snapshot, Some(snapshot));
        assert_eq!(
            restored.core.events.iter().cloned().collect::<Vec<_>>(),
            events
        );
        assert_eq!(
            restored
                .core
                .admin_history
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            expected_admin
        );
        assert!(restored.core.admin_history_complete);
        assert_eq!(
            restored.core.open_presence,
            BTreeSet::from([participant.id])
        );
    }

    #[test]
    fn checkpoint_split_handles_a_single_multibyte_char_snapshot() {
        // Regression (Codex review on PR #5316): when retained events force
        // a split, the snapshot itself can be one multibyte character; the
        // splitter's midpoint search must not floor to an empty prefix and
        // give up on the character's only valid endpoint.
        let mut core = world(16);
        core.active_module_hash = Some("h".to_owned());
        core.sequence = 3;
        let events: Vec<WorldEvent> = (1..=3)
            .map(|sequence| WorldEvent {
                sequence,
                participant_id: 0,
                kind: WorldEventKind::Chat(format!("{}-{sequence}", "x".repeat(70_000))),
            })
            .collect();
        core.events = events.clone().into();
        core.admin_history = events.clone().into();
        core.admin_history_complete = true;

        let entries = core.checkpoint_entries("\u{1f9ed}".to_owned()).unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::CheckpointSnapshotPart { .. }))
        );
        assert!(entries.iter().all(|entry| {
            serde_json::to_vec(entry).is_ok_and(|bytes| bytes.len() <= MAX_CHECKPOINT_PART_BYTES)
        }));
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.program.and_then(|program| program.snapshot),
            Some("\u{1f9ed}".to_owned())
        );
    }

    #[test]
    fn restore_rejects_incomplete_duplicate_reordered_or_mismatched_checkpoint_parts() {
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        let entries = core
            .checkpoint_entries("s".repeat(MAX_CHECKPOINT_PART_BYTES + 32))
            .unwrap();
        let snapshot_indices: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                matches!(entry, JournalEntry::CheckpointSnapshotPart { .. }).then_some(index)
            })
            .collect();
        assert_eq!(snapshot_indices.len(), 2);

        let mut missing = entries.clone();
        missing.remove(snapshot_indices[1]);
        assert!(WorldCore::restore("test-world", WorldLimits::default(), missing).is_err());

        let mut duplicate = entries.clone();
        duplicate.insert(snapshot_indices[1], duplicate[snapshot_indices[0]].clone());
        let error =
            WorldCore::restore("test-world", WorldLimits::default(), duplicate).unwrap_err();
        assert!(
            error.to_string().contains("duplicate or out of order"),
            "{error:#}"
        );

        let mut reordered = entries.clone();
        reordered.swap(snapshot_indices[0], snapshot_indices[1]);
        let error =
            WorldCore::restore("test-world", WorldLimits::default(), reordered).unwrap_err();
        assert!(
            error.to_string().contains("duplicate or out of order"),
            "{error:#}"
        );

        let mut mismatched_count = entries.clone();
        if let JournalEntry::CheckpointSnapshotPart { part_count, .. } =
            &mut mismatched_count[snapshot_indices[1]]
        {
            *part_count += 1;
        }
        let error =
            WorldCore::restore("test-world", WorldLimits::default(), mismatched_count).unwrap_err();
        assert!(
            error.to_string().contains("part_count does not match"),
            "{error:#}"
        );

        let mut mismatched_sequence = entries;
        if let JournalEntry::CheckpointSnapshotPart { sequence, .. } =
            &mut mismatched_sequence[snapshot_indices[1]]
        {
            *sequence += 1;
        }
        let error = WorldCore::restore("test-world", WorldLimits::default(), mismatched_sequence)
            .unwrap_err();
        assert!(
            error.to_string().contains("part sequence does not match"),
            "{error:#}"
        );
    }

    #[test]
    fn recorded_provenance_survives_checkpoint_compaction_and_restore() {
        let provenance = sample_source_provenance();
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.active_module_provenance = Some(provenance.clone());
        core.active_seed = 7;
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();

        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.core.active_module_provenance,
            Some(provenance.clone())
        );
        assert_eq!(restored.program.unwrap().provenance, Some(provenance));
    }

    fn sample_bundle_provenance() -> ModuleProvenance {
        let author = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let bundle = crate::program_bundle::export(
            b"bundle payload".to_vec(),
            crate::program_bundle::ProgramSpec {
                name: "chess",
                version: 1,
                licence: None,
                description: None,
                categories: &[],
            },
            &author,
        )
        .unwrap();
        ModuleProvenance::Bundle(bundle.provenance())
    }

    fn install_event(provenance: Option<ModuleProvenance>) -> WorldEvent {
        WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 1,
                provenance,
                trust: None,
            },
        }
    }

    /// #8280: an older binary cannot parse bundle provenance, so every entry
    /// that carries it raises the header to the program-bundle version, and
    /// nothing else does.
    #[test]
    fn bundle_provenance_requires_the_program_bundle_journal_version() {
        let bundle = sample_bundle_provenance();
        let committed = |provenance| JournalEntry::Committed {
            event: install_event(provenance),
        };
        assert_eq!(
            journal_version_required_by(&committed(Some(bundle.clone()))),
            JOURNAL_VERSION_PROGRAM_BUNDLE
        );
        assert_eq!(
            journal_version_required_by(&committed(Some(ModuleProvenance::Artifact))),
            JOURNAL_VERSION
        );
        assert_eq!(
            journal_version_required_by(&committed(None)),
            JOURNAL_VERSION
        );
        let part = JournalEntry::CheckpointEventsPart {
            sequence: 1,
            part_index: 0,
            part_count: 1,
            events: vec![install_event(Some(bundle))],
        };
        assert_eq!(
            journal_version_required_by(&part),
            JOURNAL_VERSION_PROGRAM_BUNDLE
        );
        let plain_part = JournalEntry::CheckpointEventsPart {
            sequence: 1,
            part_index: 0,
            part_count: 1,
            events: vec![install_event(None)],
        };
        assert_eq!(
            journal_version_required_by(&plain_part),
            JOURNAL_VERSION_CHECKPOINT_PARTS
        );
    }

    #[test]
    fn bundle_provenance_survives_checkpoint_compaction_and_restore() {
        let provenance = sample_bundle_provenance();
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.active_module_provenance = Some(provenance.clone());
        core.active_seed = 7;
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(
            entries.iter().any(|entry| matches!(
                entry,
                JournalEntry::Created { version, .. } if *version == JOURNAL_VERSION_PROGRAM_BUNDLE
            )),
            "the compacted header carries the program-bundle version: {entries:?}"
        );

        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.core.active_module_provenance,
            Some(provenance.clone())
        );
        assert_eq!(restored.program.unwrap().provenance, Some(provenance));
    }

    /// A program driven by numeric inputs: each input switches its `wants`
    /// script, and every host event is recorded. Request ids embed the input
    /// count, so re-sending an input asks again.
    struct Mock {
        script: u64,
        received: Vec<String>,
        asks: u64,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                script: 0,
                received: Vec::new(),
                asks: 0,
            }
        }
    }

    impl WorldProgram for Mock {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            match &event.kind {
                WorldEventKind::Input(bytes) => {
                    let text = std::str::from_utf8(bytes)?;
                    self.script = text.parse().unwrap_or(self.script);
                    self.asks += 1;
                }
                WorldEventKind::EntityInput { input, .. } => {
                    let text = std::str::from_utf8(input)?;
                    self.script = text.parse().unwrap_or(self.script);
                    self.asks += 1;
                }
                WorldEventKind::System { event } => self.received.push(event.clone()),
                WorldEventKind::Chat(_)
                | WorldEventKind::ProgramInstalled { .. }
                | WorldEventKind::Notification { .. } => {}
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let id = |tag: &str| format!("{tag}-{}", self.asks);
            let document = match self.script {
                1 => serde_json::json!({
                    "v": 1,
                    "subscribe": ["players"],
                    "requests": [{"id": id("now"), "cap": "time.now"}],
                }),
                2 => serde_json::json!({
                    "requests": [{"id": id("now"), "cap": "time.now"}],
                }),
                3 => serde_json::json!({
                    "requests": [{
                        "id": id("say"),
                        "cap": "chat.say",
                        "args": {"text": "hello from the program"},
                    }],
                }),
                4 => serde_json::json!({"subscribe": ["time.tick:20"]}),
                5 => serde_json::json!({
                    "requests": [{
                        "id": id("hist"),
                        "cap": "chat.history",
                        "args": {"count": 5},
                    }],
                }),
                _ => serde_json::json!({}),
            };
            Ok(Some(document.to_string()))
        }
    }

    async fn caps_world_sync(grant_on_use: bool) -> (WorldHandle, JoinCapability) {
        let limits = WorldLimits {
            grant_on_use,
            ..WorldLimits::default()
        };
        let core = WorldCore::new("caps", limits).unwrap();
        let handle = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
        (handle, token)
    }

    /// A fuel-rate admission refusal is recoverable at the actor level
    /// (#1067, Codex review on #12557): exhausting the budget must not
    /// mark the program unhealthy, so the same input succeeds once the
    /// window rolls instead of requiring a program replacement.
    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_fuel_rate_refusal_keeps_the_actor_healthy() {
        use crate::sandbox::{Sandbox, SandboxLimits};

        // The same echo program as the sandbox tests: `plaza_update`
        // returns the participant id and never traps.
        const ECHO: &str = r#"
        (module
          (memory (export "memory") 1 1)
          (data (i32.const 32) "hello")
          (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_free") (param i32 i32))
          (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
          (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
            local.get 0)
          (func (export "plaza_view") (param i32) (result i32)
            (i32.const 32))
          (func (export "plaza_out_len") (result i32) (i32.const 5)))
    "#;
        let limits = SandboxLimits {
            fuel: 10_000,
            ..SandboxLimits::default()
        };
        let wasm = wat::parse_str(ECHO).unwrap();
        let runner = Sandbox::compile(&wasm, limits).unwrap();
        let mut program = runner.instantiate(123).unwrap();
        // One allowance exactly fills the account's window.
        program.set_fuel_rate_budget(id_caps::account_quota::FuelRateBudget::new(limits.fuel));
        let core = WorldCore::new("fuel-rate", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(program));
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world.input(token.clone(), b"event".to_vec()).await.unwrap();
        // The next input is refused at admission — but the actor stays
        // healthy: the error names the fuel rate, not an unavailable
        // program.
        let refusal = world
            .input(token.clone(), b"event".to_vec())
            .await
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("fuel rate"), "{refusal}");
        assert!(
            !refusal.contains("unavailable after a previous failure"),
            "{refusal}"
        );
        // Once the window rolls, the same input succeeds: the refusal
        // did not poison the program.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        world.input(token, b"event".to_vec()).await.unwrap();
    }

    #[tokio::test]
    async fn an_ungranted_request_is_refused_and_counted() {
        let (world, token) = caps_world_sync(false).await;
        world.input(token, b"1".to_vec()).await.unwrap();
        // The program hears the refusal as a result event.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("time.now")
                .is_some_and(|use_| use_.denied > 0)
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no refusal recorded");
            tokio::task::yield_now().await;
        }
        let report = world.caps_report().await.unwrap();
        assert_eq!(report.policy, "deny");
        assert!(
            report.missing.contains(&"time.now".to_owned()),
            "{report:?}"
        );
    }

    /// #796 item 9-5 oracle: an ungranted read returns only cap_denied,
    /// while the identical granted read returns the seeded history line.
    /// Both paths must account for exactly one request in the ledger.
    async fn assert_chat_history_policy_result(granted: bool) {
        struct HistoryProbe(Arc<Mutex<Vec<serde_json::Value>>>);

        impl WorldProgram for HistoryProbe {
            fn update(&mut self, event: &WorldEvent) -> Result<()> {
                if let WorldEventKind::System { event } = &event.kind {
                    let value: serde_json::Value = serde_json::from_str(event)?;
                    if value["cap"] == "result" && value["id"] == "history-proof" {
                        self.0.lock().unwrap().push(value);
                    }
                }
                Ok(())
            }

            fn wants(&mut self) -> Result<Option<String>> {
                // Keep the same request visible for the capability report;
                // the actor must deduplicate its already answered id.
                let requests = serde_json::json!([{
                    "id": "history-proof", "cap": "chat.history", "args": {"count": 1}
                }]);
                Ok(Some(serde_json::json!({"requests": requests}).to_string()))
            }
        }

        let mut core = world(16);
        if granted {
            core.ledger
                .set_granted(BTreeSet::from(["chat.history".to_owned()]));
        }
        core.events.push_back(WorldEvent {
            sequence: 1,
            participant_id: 7,
            kind: WorldEventKind::Chat("history-proof-line".to_owned()),
        });
        let results = Arc::new(Mutex::new(Vec::new()));
        let handle = WorldHandle::spawn_with_program(core, Box::new(HistoryProbe(results.clone())));
        let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
        handle.input(token.clone(), b"read".to_vec()).await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while results.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "no history result delivered");
            tokio::task::yield_now().await;
        }
        // A second evaluation must not count or deliver the answered request again.
        handle.input(token, b"read".to_vec()).await.unwrap();
        let report = handle.caps_report().await.unwrap();

        let results = results.lock().unwrap();
        assert_eq!(results.len(), 1, "one result must reach the program");
        let result = &results[0];
        assert_eq!(report.policy, "deny");
        let usage = report.usage["chat.history"];
        if granted {
            assert_eq!(result["ok"], true, "{result}");
            let lines = result["value"]["lines"].as_array().unwrap();
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0]["text"], "history-proof-line");
            assert_eq!(usage.allowed, 1);
            assert_eq!(usage.denied, 0);
            assert!(!report.missing.contains(&"chat.history".to_owned()));
        } else {
            assert_eq!(
                result,
                &serde_json::json!({
                    "cap": "result", "id": "history-proof", "ok": false, "error": "cap_denied"
                }),
                "denial must contain no history value or lines"
            );
            assert_eq!(usage.denied, 1);
            assert_eq!(usage.allowed, 0);
            assert!(report.missing.contains(&"chat.history".to_owned()));
        }
    }

    #[tokio::test]
    async fn chat_history_policy_ungranted_is_refused_counted_and_delivered() {
        assert_chat_history_policy_result(false).await;
    }

    #[tokio::test]
    async fn chat_history_policy_granted_delivers_history_and_counts_once() {
        assert_chat_history_policy_result(true).await;
    }

    #[tokio::test]
    async fn a_granted_request_gets_a_real_answer() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["time.now".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"2".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if world.caps_report().await.unwrap().usage["time.now"].allowed > 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no answer recorded");
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn chat_history_returns_the_newest_lines_oldest_first() {
        let mut core = world(16);
        for (sequence, participant, text) in
            [(1u64, 1u64, "first"), (2, 28, "second"), (3, 1, "third")]
        {
            core.events.push_back(WorldEvent {
                sequence,
                participant_id: participant,
                kind: WorldEventKind::Chat(text.to_owned()),
            });
        }
        let newest = chat_history(&core.events, 2);
        assert_eq!(newest.len(), 2);
        assert_eq!(newest[0]["text"], "second");
        assert_eq!(newest[0]["sequence"], 2);
        assert_eq!(newest[1]["text"], "third");
        let all = chat_history(&core.events, 10);
        assert_eq!(all.len(), 3);
        assert_eq!(all[0]["text"], "first");
        assert_eq!(all[2]["text"], "third");
        assert!(chat_history(&core.events, 0).is_empty());
    }

    /// Asks the bounded-read questions of #796 item 9-5 once each and
    /// captures every result event into its records for the test to read.
    #[derive(Debug, Default)]
    struct BoundsProbe {
        answered: BTreeSet<String>,
        results: BTreeMap<String, serde_json::Value>,
    }

    impl WorldProgram for BoundsProbe {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                let value: serde_json::Value =
                    serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                if value["cap"] == "result"
                    && let Some(id) = value["id"].as_str()
                {
                    self.answered.insert(id.to_owned());
                    self.results.insert(id.to_owned(), value);
                }
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let requests: Vec<serde_json::Value> = [
                ("records-oversized", "records.read", serde_json::json!({})),
                ("hist-default", "chat.history", serde_json::json!({})),
                (
                    "hist-clamped",
                    "chat.history",
                    serde_json::json!({"count": 100_000}),
                ),
                (
                    "hist-u32-overflow",
                    "chat.history",
                    serde_json::json!({"count": 4_294_967_296_u64}),
                ),
                (
                    "hist-string",
                    "chat.history",
                    serde_json::json!({"count": "2"}),
                ),
                (
                    "hist-negative",
                    "chat.history",
                    serde_json::json!({"count": -1}),
                ),
                (
                    "hist-fractional",
                    "chat.history",
                    serde_json::json!({"count": 1.5}),
                ),
                (
                    "hist-null",
                    "chat.history",
                    serde_json::json!({"count": null}),
                ),
                (
                    "hist-object",
                    "chat.history",
                    serde_json::json!({"count": {"n": 2}}),
                ),
            ]
            .into_iter()
            .filter(|(id, _, _)| !self.answered.contains(*id))
            .map(|(id, cap, args)| serde_json::json!({"id": id, "cap": cap, "args": args}))
            .collect();
            Ok(Some(
                serde_json::json!({"v": 1, "requests": requests}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"results": self.results}).to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn chat_history_defaults_and_clamps_and_records_read_rejects_an_oversized_first_record() {
        let mut core = world(128);
        core.ledger.set_granted(BTreeSet::from([
            "chat.history".to_owned(),
            "records.read".to_owned(),
        ]));
        for sequence in 1..=70_u64 {
            core.events.push_back(WorldEvent {
                sequence,
                participant_id: 1,
                kind: WorldEventKind::Chat(format!("line-{sequence}")),
            });
        }
        let mut actor = WorldActor::for_test(core, Box::new(BoundsProbe::default()), None);
        // A single record larger than one records page can only reach the
        // actor from a store that never passed parse_records; inject it
        // straight into the published-records channel to prove the read
        // fails closed instead of serving a truncated page.
        let mut oversized = Records::new();
        oversized.insert(
            "big".to_owned(),
            serde_json::Value::String("x".repeat(crate::world_session::MAX_RECORDS_PAGE_BYTES + 1)),
        );
        actor.records_sender.send_replace((0, Arc::new(oversized)));

        actor.after_change().await;

        let records = actor.records_sender.borrow().1.clone();
        let results = &records["results"];
        // chat.history with no count serves the default 16 newest lines,
        // oldest first.
        let default = &results["hist-default"];
        assert_eq!(default["ok"], true, "{default}");
        let lines = default["value"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 16);
        assert_eq!(lines[0]["text"], "line-55");
        assert_eq!(lines[15]["text"], "line-70");
        // An over-large count clamps to MAX_CHAT_HISTORY_LINES.
        let clamped = &results["hist-clamped"];
        assert_eq!(clamped["ok"], true, "{clamped}");
        let lines = clamped["value"]["lines"].as_array().unwrap();
        assert_eq!(lines.len(), crate::world_caps::MAX_CHAT_HISTORY_LINES);
        assert_eq!(lines[0]["text"], "line-7");
        assert_eq!(lines[63]["text"], "line-70");
        let large = &results["hist-u32-overflow"];
        assert_eq!(large["ok"], true, "{large}");
        assert_eq!(large["value"]["lines"], clamped["value"]["lines"]);
        // A supplied count of the wrong JSON kind is rejected, never
        // treated as omitted (#7892).
        for id in [
            "hist-string",
            "hist-negative",
            "hist-fractional",
            "hist-null",
            "hist-object",
        ] {
            let malformed = &results[id];
            assert_eq!(malformed["ok"], false, "{id}: {malformed}");
            assert_eq!(malformed["error"], "bad_args", "{id}: {malformed}");
        }
        // records.read whose first record cannot fit one page is BadArgs,
        // never an empty page that looks complete.
        let oversized_result = &results["records-oversized"];
        assert_eq!(oversized_result["ok"], false, "{oversized_result}");
        assert_eq!(oversized_result["error"], "bad_args");
    }

    /// #7899 regression probe: asks records.read with malformed and
    /// well-formed prefix/after shapes and captures every result.
    #[derive(Debug, Default)]
    struct RecordsReadShapeProbe {
        answered: BTreeSet<String>,
        results: BTreeMap<String, serde_json::Value>,
    }

    impl WorldProgram for RecordsReadShapeProbe {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                let value: serde_json::Value =
                    serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                if value["cap"] == "result"
                    && let Some(id) = value["id"].as_str()
                {
                    self.answered.insert(id.to_owned());
                    self.results.insert(id.to_owned(), value);
                }
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let requests: Vec<serde_json::Value> = [
                ("omitted", serde_json::json!({})),
                ("valid-prefix", serde_json::json!({"prefix": "co"})),
                ("valid-after", serde_json::json!({"after": "count"})),
                ("prefix-number", serde_json::json!({"prefix": 123})),
                ("prefix-null", serde_json::json!({"prefix": null})),
                ("prefix-object", serde_json::json!({"prefix": {"x": 1}})),
                ("prefix-array", serde_json::json!({"prefix": ["co"]})),
                ("prefix-bool", serde_json::json!({"prefix": true})),
                ("after-number", serde_json::json!({"after": 123})),
                ("after-null", serde_json::json!({"after": null})),
                ("after-object", serde_json::json!({"after": {"x": 1}})),
            ]
            .into_iter()
            .filter(|(id, _)| !self.answered.contains(*id))
            .map(|(id, args)| serde_json::json!({"id": id, "cap": "records.read", "args": args}))
            .collect();
            Ok(Some(
                serde_json::json!({"v": 1, "requests": requests}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            // The probe publishes the dataset itself: the actor
            // replaces the published records with the program's own
            // records() output between wants rounds, so a dataset
            // injected from outside would not survive to the later
            // requests. The keys are constant across rounds, which
            // keeps every control deterministic.
            Ok(Some(
                serde_json::json!({
                    "count": {"n": 1},
                    "other": {"n": 1},
                    "zebra": {"n": 1},
                    "results": self.results,
                })
                .to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn records_read_rejects_a_non_string_prefix_or_after() {
        // Oracle: issue #7899 — a supplied non-string prefix/after is
        // bad_args, never a silent "omitted" that widens the read to
        // every record or restarts pagination; omitted and string
        // shapes behave exactly as before.
        let mut core = world(128);
        core.ledger
            .set_granted(BTreeSet::from(["records.read".to_owned()]));
        let mut actor =
            WorldActor::for_test(core, Box::new(RecordsReadShapeProbe::default()), None);

        actor.after_change().await;

        let records = actor.records_sender.borrow().1.clone();
        let results = &records["results"];
        let keys = |id: &str| -> Vec<String> {
            results[id]["value"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|entry| entry["key"].as_str().map(str::to_owned))
                .collect()
        };
        // Controls: omitted reads everything, a string prefix scopes,
        // a string after continues from the cursor.
        assert_eq!(results["omitted"]["ok"], true, "{}", results["omitted"]);
        assert_eq!(keys("omitted"), ["count", "other", "results", "zebra"]);
        assert_eq!(keys("valid-prefix"), ["count"]);
        assert_eq!(keys("valid-after"), ["other", "results", "zebra"]);
        // Every present-but-non-string shape is bad_args.
        for id in [
            "prefix-number",
            "prefix-null",
            "prefix-object",
            "prefix-array",
            "prefix-bool",
            "after-number",
            "after-null",
            "after-object",
        ] {
            assert_eq!(results[id]["ok"], false, "{id}: {}", results[id]);
            assert_eq!(results[id]["error"], "bad_args", "{id}: {}", results[id]);
        }
    }

    /// #7942: `device.info` driven through the public `WorldHandle`
    /// spawn path — the manifest supplied at spawn comes back in the
    /// result event, twice (the cached copy serves repeated requests;
    /// nothing re-enumerates), with no direct field mutation.
    #[tokio::test]
    async fn device_info_through_the_public_spawn_path_returns_the_supplied_manifest() {
        use std::sync::Mutex;

        #[derive(Debug, Default)]
        struct DeviceProbe {
            results: Arc<Mutex<Vec<serde_json::Value>>>,
        }

        impl WorldProgram for DeviceProbe {
            fn snapshot(&mut self) -> Result<Option<String>> {
                Ok(Some("{}".to_owned()))
            }

            fn update(&mut self, event: &WorldEvent) -> Result<()> {
                if let WorldEventKind::System { event } = &event.kind {
                    let value: serde_json::Value =
                        serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                    if value["cap"] == "result" {
                        self.results.lock().unwrap().push(value);
                    }
                }
                Ok(())
            }

            fn wants(&mut self) -> Result<Option<String>> {
                Ok(Some(
                    serde_json::json!({
                        "v": 1,
                        "requests": [
                            {"id": "device-1", "cap": "device.info", "args": {}},
                            {"id": "device-2", "cap": "device.info", "args": {}},
                        ],
                    })
                    .to_string(),
                ))
            }
        }

        let manifest = crate::world_caps::DeviceManifest {
            display: serde_json::json!({"kind": "pixel", "width": 128, "height": 64, "colors": 1}),
            input: serde_json::json!({"keys": 8, "encoder": true, "pointer": false, "text_entry": "none"}),
            sensors: serde_json::json!({"gnss": {"accuracy_m": 5}, "microphone": false, "camera": false}),
            audio: serde_json::json!({"out": "pcm", "in": null}),
            network: serde_json::json!({"transport": "mqtt", "rtt_class": "slow"}),
        };
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["device.info".to_owned()]));
        let results = Arc::new(Mutex::new(Vec::new()));
        let probe = DeviceProbe {
            results: Arc::clone(&results),
        };
        let _world =
            WorldHandle::spawn_with_device_manifest(core, Box::new(probe), Some(manifest.clone()));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if results.lock().unwrap().len() >= 2 {
                break;
            }
            assert!(Instant::now() < deadline, "device.info was not answered");
            tokio::task::yield_now().await;
        }
        let results = results.lock().unwrap();
        assert_eq!(results.len(), 2);
        for result in results.iter() {
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(result["value"], manifest.to_json(), "{result}");
        }
    }

    #[tokio::test]
    async fn records_read_pages_fit_the_result_event_that_delivers_them() {
        // Regression for #4888: sixteen ~8 KiB records total ~128 KiB,
        // which the old 768 KiB page admission accepted whole — and
        // result_event then replaced the entire page with a
        // result_too_large error above 64 KiB, so the program never saw
        // a single record. The page must now stop at the cap-result
        // bound and deliver records, with next_after naming the rest.
        // Captured results are shared with the test through a handle, not
        // republished as records: a full page result is ~64 KiB, and
        // parse_records caps one record value at MAX_RECORD_VALUE_BYTES
        // (8 KiB), so a records readout makes deliver() refuse the
        // publication and the test would read an empty channel (#5128).
        // The delivery under test is the result event reaching update().
        #[derive(Debug, Default)]
        struct PageProbe {
            answered: BTreeSet<String>,
            results: Arc<std::sync::Mutex<BTreeMap<String, serde_json::Value>>>,
        }

        impl WorldProgram for PageProbe {
            fn update(&mut self, event: &WorldEvent) -> Result<()> {
                if let WorldEventKind::System { event } = &event.kind {
                    let value: serde_json::Value =
                        serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                    if value["cap"] == "result"
                        && let Some(id) = value["id"].as_str()
                    {
                        self.answered.insert(id.to_owned());
                        self.results.lock().unwrap().insert(id.to_owned(), value);
                    }
                }
                Ok(())
            }

            fn wants(&mut self) -> Result<Option<String>> {
                let requests: Vec<serde_json::Value> = [(
                    "records-page",
                    "records.read",
                    serde_json::json!({}),
                )]
                .into_iter()
                .filter(|(id, _, _)| !self.answered.contains(*id))
                .map(|(id, cap, args)| serde_json::json!({"id": id, "cap": cap, "args": args}))
                .collect();
                Ok(Some(
                    serde_json::json!({"v": 1, "requests": requests}).to_string(),
                ))
            }
        }

        let mut core = world(128);
        core.ledger
            .set_granted(BTreeSet::from(["records.read".to_owned()]));
        let captured: Arc<std::sync::Mutex<BTreeMap<String, serde_json::Value>>> =
            Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let probe = PageProbe {
            results: captured.clone(),
            ..Default::default()
        };
        let mut actor = WorldActor::for_test(core, Box::new(probe), None);
        let mut injected = Records::new();
        for index in 0..16 {
            injected.insert(
                format!("r{index:02}"),
                serde_json::Value::String("x".repeat(8000)),
            );
        }
        actor.records_sender.send_replace((0, Arc::new(injected)));

        actor.after_change().await;

        let captured = captured.lock().unwrap();
        let page = &captured["records-page"];
        assert_eq!(page["ok"], true, "{page}");
        let delivered = page["value"]["records"].as_array().unwrap();
        // Each serialized entry is 8025 bytes with its comma; the
        // 64512-byte cap-result bound admits exactly 8 of the 16.
        assert_eq!(delivered.len(), 8, "{page}");
        assert_eq!(delivered[0]["key"], "r00");
        assert_eq!(page["value"]["next_after"], "r07");
    }

    #[tokio::test]
    async fn a_granted_chat_history_request_is_served_over_the_ledger() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(
                vec!["chat.say".to_owned(), "chat.history".to_owned()],
                Vec::new(),
            )
            .await
            .unwrap();
        // The program says one line first, then asks for the history.
        world.input(token.clone(), b"3".to_vec()).await.unwrap();
        world.input(token, b"5".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("chat.history")
                .is_some_and(|use_| use_.allowed > 0)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no history answer recorded: {report:?}"
            );
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn chat_say_commits_a_system_chat_line_that_sessions_see() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        let mut events = world.subscribe();
        world.input(token, b"3".to_vec()).await.unwrap();
        // The participant's own input event arrives first; keep waiting for
        // the host's chat line.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), events.recv())
                .await
                .expect("no chat event")
                .unwrap();
            if let WorldEventKind::Chat(text) = &event.kind {
                assert_eq!(text, "hello from the program");
                assert_eq!(event.participant_id, 0, "the host said it");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "chat never arrived");
        }
    }

    #[tokio::test]
    async fn a_participant_cannot_make_the_program_do_what_its_scopes_forbid() {
        let core = WorldCore::new("caps", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = world
            .issue("ann", WorldScopes::JOIN.union(WorldScopes::INPUT))
            .await
            .unwrap();
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"3".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("chat.say")
                .is_some_and(|use_| use_.denied > 0)
            {
                assert_eq!(report.usage["chat.say"].allowed, 0);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no refusal recorded");
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_participant_with_chat_lets_the_program_say_what_it_asks() {
        let core = WorldCore::new("caps", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"3".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("chat.say")
                .is_some_and(|use_| use_.allowed > 0)
            {
                assert_eq!(report.usage["chat.say"].denied, 0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no allowed use recorded"
            );
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn grant_on_use_grants_what_the_program_actually_uses() {
        let (world, token) = caps_world_sync(true).await;
        world.input(token, b"1".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if world.caps_report().await.unwrap().usage["time.now"].allowed > 0 {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "never granted");
            tokio::task::yield_now().await;
        }
        let report = world.caps_report().await.unwrap();
        assert_eq!(report.policy, "grant-on-use");
        assert!(report.granted.contains(&"time.now".to_owned()));
        assert!(report.missing.is_empty());
    }

    #[test]
    fn system_events_and_grants_survive_a_restore() {
        let secret = "ab".repeat(32);
        let token = JoinCapability::from_wire(format!("1.{secret}"));
        let digest = hex_encode(&token_digest(token.expose()));
        let entries = vec![
            JournalEntry::Created {
                world_id: "caps".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::CapsGranted {
                granted: vec!["time.now".to_owned()],
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest,
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            },
            JournalEntry::System {
                event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: r#"{"cap":"result","id":"now-1","ok":true,"value":{"now":1}}"#
                            .to_owned(),
                    },
                },
            },
        ];
        let restored = WorldCore::restore("caps", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
        assert_eq!(restored.core.sequence, 0, "system events carry no sequence");
        assert_eq!(restored.replay.len(), 1, "the program must see it again");
        assert!(
            restored.core.authorize(&token, WorldScopes::JOIN).is_ok(),
            "capabilities restore after a grant entry"
        );
    }

    fn issued_entry_for(participant_id: u64, scopes: u8) -> JournalEntry {
        let secret = "ef".repeat(32);
        let digest = hex_encode(&token_digest(&format!("{participant_id}.{secret}")));
        JournalEntry::Issued {
            issued_at: None,
            participant: Participant {
                id: participant_id,
                display_name: format!("ann-{participant_id}"),
            },
            digest,
            scopes,
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        }
    }

    fn issued_entry(scopes: u8) -> JournalEntry {
        issued_entry_for(1, scopes)
    }

    fn restore_author_entries(entries: Vec<JournalEntry>) -> anyhow::Result<RestoredWorld> {
        WorldCore::restore(
            "restore-authors",
            WorldLimits::default(),
            std::iter::once(JournalEntry::Created {
                world_id: "restore-authors".to_owned(),
                version: JOURNAL_VERSION,
            })
            .chain(entries),
        )
    }

    fn committed_event(sequence: u64, participant_id: u64, kind: WorldEventKind) -> JournalEntry {
        JournalEntry::Committed {
            event: WorldEvent {
                sequence,
                participant_id,
                kind,
            },
        }
    }

    fn checkpoint_event(sequence: u64, events: Vec<WorldEvent>) -> JournalEntry {
        JournalEntry::Checkpoint {
            sequence,
            module_hash: "module".to_owned(),
            seed: 0,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            snapshot: "snapshot".to_owned(),
            events,
            admin_history: None,
            lease: None,
            open_presence: None,
        }
    }

    #[test]
    fn restore_refuses_committed_events_from_never_issued_participants() {
        for kind in [
            WorldEventKind::Input(b"ghost".to_vec()),
            WorldEventKind::Chat("ghost".to_owned()),
        ] {
            let error = restore_author_entries(vec![committed_event(1, 99, kind)]).unwrap_err();
            assert!(
                error.to_string().contains("before it was issued"),
                "error explains the missing prior issuance: {error}"
            );
        }
    }

    #[test]
    fn restore_refuses_checkpoint_events_from_never_issued_participants() {
        let error = restore_author_entries(vec![checkpoint_event(
            1,
            vec![WorldEvent {
                sequence: 1,
                participant_id: 99,
                kind: WorldEventKind::Input(b"ghost".to_vec()),
            }],
        )])
        .unwrap_err();
        assert!(
            error.to_string().contains("before it was issued"),
            "error explains the missing prior issuance: {error}"
        );
    }

    #[test]
    fn restore_refuses_events_from_participants_issued_later() {
        let error = restore_author_entries(vec![
            committed_event(1, 2, WorldEventKind::Input(b"late".to_vec())),
            issued_entry_for(2, WorldScopes::GUEST.bits()),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("before it was issued"),
            "error explains the missing prior issuance: {error}"
        );
    }

    #[test]
    fn restore_refuses_committed_events_after_author_revocation() {
        let error = restore_author_entries(vec![
            issued_entry(WorldScopes::GUEST.bits()),
            JournalEntry::Revoked {
                revoked_at: None,
                participant_id: 1,
                by: None,
            },
            committed_event(1, 1, WorldEventKind::Chat("after revoke".to_owned())),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("after revocation"),
            "error explains the earlier revocation: {error}"
        );
    }

    #[test]
    fn restore_refuses_input_and_chat_without_the_matching_scope() {
        for (kind, scope_name) in [
            (WorldEventKind::Input(b"move".to_vec()), "INPUT"),
            (WorldEventKind::Chat("hello".to_owned()), "CHAT"),
        ] {
            let error = restore_author_entries(vec![
                issued_entry(WorldScopes::JOIN.bits()),
                committed_event(1, 1, kind),
            ])
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(&format!("without {scope_name} scope")),
                "error explains the missing scope: {error}"
            );
        }
    }

    #[test]
    fn restore_refuses_checkpoint_events_without_the_matching_scope() {
        let error = restore_author_entries(vec![
            issued_entry(WorldScopes::JOIN.bits()),
            checkpoint_event(
                1,
                vec![WorldEvent {
                    sequence: 1,
                    participant_id: 1,
                    kind: WorldEventKind::Chat("unauthorized".to_owned()),
                }],
            ),
        ])
        .unwrap_err();
        assert!(
            error.to_string().contains("without CHAT scope"),
            "error explains the missing scope: {error}"
        );
    }

    #[test]
    fn restore_accepts_host_chat_and_committed_content_before_later_revocation() {
        let host_chat = restore_author_entries(vec![committed_event(
            1,
            0,
            WorldEventKind::Chat("host".to_owned()),
        )])
        .unwrap();
        assert!(matches!(
            host_chat.core.events.back().map(|event| &event.kind),
            Some(WorldEventKind::Chat(text)) if text == "host"
        ));

        let restored = restore_author_entries(vec![
            issued_entry(WorldScopes::GUEST.bits()),
            committed_event(1, 1, WorldEventKind::Chat("before revoke".to_owned())),
            JournalEntry::Revoked {
                revoked_at: None,
                participant_id: 1,
                by: None,
            },
        ])
        .unwrap();
        assert!(matches!(
            restored.core.events.back().map(|event| &event.kind),
            Some(WorldEventKind::Chat(text)) if text == "before revoke"
        ));
    }

    #[test]
    fn checkpoint_restore_keeps_valid_history_from_later_revoked_participant() {
        let mut core = world(8);
        let (participant, token) = core
            .issue_capability("Ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        core.input(&token, b"before revoke").unwrap();
        assert_eq!(core.revoke_tree(participant.id), vec![participant.id]);
        core.active_module_hash = Some("module".to_owned());

        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert!(matches!(
            restored.core.events.back(),
            Some(WorldEvent {
                participant_id,
                kind: WorldEventKind::Input(input),
                ..
            }) if *participant_id == participant.id && input.as_slice() == b"before revoke"
        ));
    }

    #[test]
    fn restore_accepts_friend_delegated_history_without_directory_state() {
        // Codex P1 on PR #4518: restore runs before the directory loads,
        // so scope validation must use the recorded capability chain.
        // A participant delegated by a parent with a different subject
        // (the friend-delegation shape) has zero effective scopes under
        // an empty directory; its valid history must still restore.
        let secret = "ef".repeat(32);
        let issued_with =
            |id: u64, scopes: u8, parent: Option<u64>, subject: &str| JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id,
                    display_name: format!("delegate-{id}"),
                },
                digest: hex_encode(&token_digest(&format!("{id}.{secret}"))),
                scopes,
                parent,
                expires_at: None,
                uses: None,
                used: 0,
                subject: Some(subject.to_owned()),
            };
        let delegated_scopes =
            WorldScopes::JOIN.bits() | WorldScopes::INPUT.bits() | WorldScopes::CHAT.bits();
        let entries = vec![
            issued_with(1, WorldScopes::ALL.bits(), None, "subject-a"),
            issued_with(2, delegated_scopes, Some(1), "subject-b"),
            committed_event(1, 2, WorldEventKind::Input(b"hello".to_vec())),
            committed_event(2, 2, WorldEventKind::Chat("hi".to_owned())),
        ];
        let restored = restore_author_entries(entries).unwrap();
        assert!(
            restored.core.capabilities.get(&2).is_some(),
            "delegated participant restores"
        );

        let checkpoint = vec![
            issued_with(1, WorldScopes::ALL.bits(), None, "subject-a"),
            issued_with(2, delegated_scopes, Some(1), "subject-b"),
            checkpoint_event(
                1,
                vec![WorldEvent {
                    sequence: 1,
                    participant_id: 2,
                    kind: WorldEventKind::Input(b"kept".to_vec()),
                }],
            ),
        ];
        restore_author_entries(checkpoint).unwrap();
    }

    fn restore_with_scopes(scopes: u8) -> anyhow::Result<RestoredWorld> {
        WorldCore::restore(
            "scopes",
            WorldLimits::default(),
            vec![
                JournalEntry::Created {
                    world_id: "scopes".to_owned(),
                    version: JOURNAL_VERSION,
                },
                issued_entry(scopes),
            ],
        )
    }

    #[test]
    fn test_restore_refuses_issued_scopes_naming_no_scope() {
        // 0x80 names no scope; from_bits rejects the whole byte.
        let error = restore_with_scopes(0x87).unwrap_err();
        assert!(
            error.to_string().contains("participant 1"),
            "error names the participant: {error}"
        );
        assert!(
            error.to_string().contains("name no scope"),
            "error explains the refusal: {error}"
        );
    }

    #[test]
    fn test_restore_refuses_issued_scopes_without_join() {
        for scopes in [0, WorldScopes::CHAT.bits(), WorldScopes::INPUT.bits()] {
            let error = restore_with_scopes(scopes).unwrap_err();
            assert!(
                error.to_string().contains("join scope"),
                "scopes {scopes:#04x} refused for missing JOIN: {error}"
            );
        }
    }

    #[test]
    fn test_restore_accepts_guest_and_admin_bit_scopes() {
        for scopes in [
            WorldScopes::GUEST.bits(),
            WorldScopes::JOIN.bits() | WorldScopes::BUILDER.bits(),
            WorldScopes::JOIN.bits() | WorldScopes::MODERATOR.bits(),
        ] {
            let restored = restore_with_scopes(scopes).unwrap();
            let record = restored.core.capabilities.get(&1).unwrap();
            assert_eq!(record.scopes.bits(), scopes, "scopes round-trip");
        }
    }

    #[test]
    fn host_system_events_cannot_enter_participant_history() {
        let system = WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::System {
                event:
                    r#"{"cap":"result","id":"private","ok":true,"value":{"secret":"host-only"}}"#
                        .to_owned(),
            },
        };
        let committed_system = vec![
            JournalEntry::Created {
                world_id: "system-history".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Committed {
                event: system.clone(),
            },
        ];
        assert!(
            WorldCore::restore("system-history", WorldLimits::default(), committed_system).is_err(),
            "a host event cannot be restored as a sequenced participant event"
        );

        let malformed_system = vec![
            JournalEntry::Created {
                world_id: "system-history".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::System {
                event: WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::Chat("not a system event".to_owned()),
                },
            },
        ];
        assert!(
            WorldCore::restore("system-history", WorldLimits::default(), malformed_system).is_err(),
            "a journal System entry must carry a System event payload"
        );

        let mut checkpoint_core = WorldCore::new("system-history", WorldLimits::default()).unwrap();
        checkpoint_core.active_module_hash = Some("h".to_owned());
        let mut checkpoint = checkpoint_core
            .checkpoint_entries("snapshot".to_owned())
            .unwrap();
        if let Some(JournalEntry::Checkpoint {
            sequence, events, ..
        }) = checkpoint.last_mut()
        {
            *sequence = 1;
            *events = vec![system.clone()];
        }
        assert!(
            WorldCore::restore("system-history", WorldLimits::default(), checkpoint).is_err(),
            "a host event cannot be restored in sequenced checkpoint history"
        );

        let mut core = WorldCore::new("system-history", WorldLimits::default()).unwrap();
        let (_, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        core.sequence = system.sequence;
        core.events.push_back(system);

        assert!(
            core.snapshot(&token).unwrap().events.is_empty(),
            "a snapshot must filter even a malformed retained host event"
        );
        assert!(
            core.events_after(&token, 0).unwrap().events.is_empty(),
            "catch-up pages must filter even a malformed retained host event"
        );
    }

    #[test]
    fn a_restore_closes_presence_the_journal_left_open() {
        let secret = "cd".repeat(32);
        let digest = hex_encode(&token_digest(&format!("1.{secret}")));
        let presence = |joined: bool| JournalEntry::System {
            event: WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: presence_event(1, "ann", joined),
                },
            },
        };
        let entries = vec![
            JournalEntry::Created {
                world_id: "presence".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest,
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            },
            presence(true),
            presence(false),
            presence(true),
        ];
        let restored =
            WorldCore::restore("presence", WorldLimits::default(), entries.clone()).unwrap();
        let last = restored.replay.last().unwrap();
        assert_eq!(
            restored.replay.len(),
            4,
            "the open join is closed after the journal's events"
        );
        assert_eq!(
            last.kind,
            WorldEventKind::System {
                event: presence_event(1, "ann", false),
            }
        );

        let closed =
            WorldCore::restore("presence", WorldLimits::default(), entries[..4].to_vec()).unwrap();
        assert_eq!(closed.replay.len(), 2, "a closed join needs no extra leave");
    }

    #[test]
    fn a_checkpoint_keeps_the_granted_set() {
        let mut core = WorldCore::new("caps", WorldLimits::default()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.now".to_owned()]));
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::CapsGranted { granted } if granted == &vec!["time.now".to_owned()])));
        // A compacted journal restores with the grant intact.
        let restored = WorldCore::restore("caps", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
    }

    #[test]
    fn a_checkpoint_preserves_ordered_admin_history_across_host_events() {
        let mut core = WorldCore::new("history", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        let (participant, _) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let install = WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        };
        core.commit_prepared(install.clone()).unwrap();
        let joined = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(participant.id, "ann", true),
            },
        };
        core.record_admin_history(joined.clone());
        let input = WorldEvent {
            sequence: 2,
            participant_id: 1,
            kind: WorldEventKind::Input(b"move".to_vec()),
        };
        core.commit_prepared(input.clone()).unwrap();
        let tick = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: r#"{"cap":"time.tick","interval_ms":1000}"#.to_owned(),
            },
        };
        core.record_admin_history(tick.clone());
        let schedule = WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: schedule_fired_event("wake", false),
            },
        };
        core.record_admin_history(schedule.clone());

        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        let restored = WorldCore::restore("history", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.admin_history_complete);
        assert_eq!(
            restored
                .core
                .admin_history
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![
                install,
                joined,
                input,
                tick,
                schedule,
                WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: presence_event(participant.id, "ann", false),
                    },
                },
            ]
        );
    }

    #[test]
    fn a_checkpoint_preserves_open_presence_and_restore_closes_it_once() {
        let mut core = WorldCore::new("presence-checkpoint", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        let (participant, _) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let presence = |joined| WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(participant.id, "ann", joined),
            },
        };
        core.record_admin_history(presence(true));

        let checkpoint = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(matches!(
            checkpoint.first(),
            Some(JournalEntry::Created { version, .. }) if *version >= JOURNAL_VERSION_OPEN_PRESENCE
        ));
        assert!(matches!(
            checkpoint.last(),
            Some(JournalEntry::Checkpoint {
                open_presence: Some(participants),
                ..
            }) if participants == &vec![participant.id]
        ));
        let mut mislabeled = checkpoint.clone();
        if let Some(JournalEntry::Created { version, .. }) = mislabeled.first_mut() {
            *version = JOURNAL_VERSION_REGION_HANDOFF;
        }
        assert!(
            WorldCore::restore("presence-checkpoint", WorldLimits::default(), mislabeled).is_err(),
            "a v3 header cannot silently carry v4 presence semantics"
        );

        let reopened = WorldCore::restore(
            "presence-checkpoint",
            WorldLimits::default(),
            checkpoint.clone(),
        )
        .unwrap();
        assert_eq!(reopened.replay.len(), 1);
        assert_eq!(
            reopened.replay[0].kind,
            WorldEventKind::System {
                event: presence_event(participant.id, "ann", false),
            }
        );
        assert!(reopened.core.open_presence.is_empty());

        let mut already_closed = checkpoint.clone();
        already_closed.push(JournalEntry::System {
            event: presence(false),
        });
        let reopened = WorldCore::restore(
            "presence-checkpoint",
            WorldLimits::default(),
            already_closed,
        )
        .unwrap();
        assert_eq!(reopened.replay, vec![presence(false)]);

        let mut reopened_then_joined = checkpoint.clone();
        reopened_then_joined.extend([
            JournalEntry::System {
                event: presence(false),
            },
            JournalEntry::System {
                event: presence(true),
            },
        ]);
        let reopened = WorldCore::restore(
            "presence-checkpoint",
            WorldLimits::default(),
            reopened_then_joined,
        )
        .unwrap();
        assert_eq!(reopened.replay.len(), 3);
        assert_eq!(
            reopened.replay.last().unwrap().kind,
            WorldEventKind::System {
                event: presence_event(participant.id, "ann", false),
            }
        );

        // A legacy v3 checkpoint has no dedicated presence field; if its
        // complete admin history is available, the open join remains
        // recoverable. Older checkpoints without complete history still load
        // as before, without guessing about truncated joins.
        let mut legacy = checkpoint;
        if let Some(JournalEntry::Created { version, .. }) = legacy.first_mut() {
            *version = JOURNAL_VERSION_REGION_HANDOFF;
        }
        if let Some(JournalEntry::Checkpoint { open_presence, .. }) = legacy.last_mut() {
            *open_presence = None;
        }
        // A journal from before join times never carried them.
        for entry in &mut legacy {
            if let JournalEntry::Issued { issued_at, .. } = entry {
                *issued_at = None;
            }
        }
        let reopened =
            WorldCore::restore("presence-checkpoint", WorldLimits::default(), legacy).unwrap();
        assert_eq!(reopened.replay.len(), 1);
        assert_eq!(
            reopened.replay[0].kind,
            WorldEventKind::System {
                event: presence_event(participant.id, "ann", false),
            }
        );
    }

    #[test]
    fn a_legacy_checkpoint_with_unknown_presence_clears_stale_guest_state_and_stays_partial() {
        let mut core = WorldCore::new("legacy-presence", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        let (ann, _) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let (bob, _) = core
            .issue_capability("bob", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        core.commit_prepared(WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        })
        .unwrap();
        core.record_admin_history(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(ann.id, "ann", true),
            },
        });
        let checkpoint = core
            .checkpoint_entries("snapshot-with-ann-online".to_owned())
            .unwrap();

        for legacy_history in [
            None,
            Some(JournalAdminHistory {
                events: Vec::new(),
                complete: false,
            }),
        ] {
            let mut legacy = checkpoint.clone();
            if let Some(JournalEntry::Created { version, .. }) = legacy.first_mut() {
                *version = JOURNAL_VERSION_REGION_HANDOFF;
            }
            // A journal from before join times never carried them.
            for entry in &mut legacy {
                if let JournalEntry::Issued { issued_at, .. } = entry {
                    *issued_at = None;
                }
            }
            if let Some(JournalEntry::Checkpoint {
                admin_history,
                open_presence,
                ..
            }) = legacy.last_mut()
            {
                *admin_history = legacy_history;
                *open_presence = None;
            }

            let restored =
                WorldCore::restore("legacy-presence", WorldLimits::default(), legacy).unwrap();
            assert_eq!(
                restored
                    .program
                    .as_ref()
                    .and_then(|program| program.snapshot.as_deref()),
                Some("snapshot-with-ann-online"),
                "the legacy snapshot itself still contains Ann's stale online state"
            );
            assert!(
                !restored.core.admin_history_complete,
                "synthetic cleanup must never promote missing or partial history"
            );

            let changes: Vec<_> = restored
                .replay
                .iter()
                .filter_map(|event| match &event.kind {
                    WorldEventKind::System { event } => presence_change(event),
                    _ => None,
                })
                .collect();
            assert_eq!(changes, vec![(ann.id, false), (bob.id, false)]);

            // Model the guest-visible set loaded from the opaque snapshot,
            // then apply the host-only replay events that follow that snapshot.
            let mut visible_online = std::collections::BTreeSet::from([ann.id]);
            for (participant, joined) in changes {
                if joined {
                    visible_online.insert(participant);
                } else {
                    visible_online.remove(&participant);
                }
            }
            assert!(
                visible_online.is_empty(),
                "legacy recovery cannot leave stale guest-visible presence"
            );
            assert!(
                !restored.core.admin_history_complete,
                "recording cleanup events must preserve partial-history status"
            );
        }
    }

    #[test]
    fn a_checkpoint_persists_known_empty_presence_with_the_new_format_version() {
        let mut core = WorldCore::new("empty-presence", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(matches!(
            entries.first(),
            Some(JournalEntry::Created {
                version: JOURNAL_VERSION_OPEN_PRESENCE,
                ..
            })
        ));
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::Checkpoint {
                open_presence: Some(participants),
                ..
            }) if participants.is_empty()
        ));
        let restored =
            WorldCore::restore("empty-presence", WorldLimits::default(), entries).unwrap();
        assert!(restored.replay.is_empty());
        assert!(restored.core.open_presence.is_empty());
    }

    #[test]
    fn a_revoked_connected_guest_over_the_quota_still_writes_a_restorable_checkpoint() {
        let limits = WorldLimits {
            participants: 1,
            ..WorldLimits::default()
        };
        let mut core = WorldCore::new("revoked-presence", limits.clone()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        let (ann, _) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        core.record_admin_history(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(ann.id, "ann", true),
            },
        });

        // Revocation is durable immediately, but Ann's session may not have
        // processed the revocation signal and emitted `left` yet.
        assert_eq!(core.revoke_tree(ann.id), vec![ann.id]);
        let (bob, _) = core
            .issue_capability("bob", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        core.record_admin_history(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: presence_event(bob.id, "bob", true),
            },
        });
        assert_eq!(core.open_presence.len(), 2);
        assert_eq!(limits.participants, 1);

        // Presence has its own stable journal bound, not the configurable
        // active-capability quota. Every checkpoint the writer emits must be
        // accepted under the same limits, even before Ann's leave arrives.
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::Checkpoint {
                open_presence: Some(participants),
                ..
            }) if participants == &vec![ann.id, bob.id]
        ));
        let restored = WorldCore::restore("revoked-presence", limits.clone(), entries.clone())
            .expect("the checkpoint written while revoked Ann is connected must restore");
        assert_eq!(restored.replay.len(), 2);
        assert!(restored.core.open_presence.is_empty());

        // Both directions enforce the same hard format bound. A live world
        // beyond it keeps its full journal instead of producing a checkpoint
        // that a later process cannot load.
        let mut unrepresentable = WorldCore::new("too-many-presences", limits.clone()).unwrap();
        unrepresentable.active_module_hash = Some("h".to_owned());
        unrepresentable.open_presence = (0..=MAX_CHECKPOINT_OPEN_PRESENCE as u64).collect();
        assert!(
            unrepresentable
                .checkpoint_entries("snapshot".to_owned())
                .is_err()
        );

        let mut oversized_checkpoint = entries;
        if let Some(JournalEntry::Checkpoint { open_presence, .. }) =
            oversized_checkpoint.last_mut()
        {
            *open_presence = Some((0..=MAX_CHECKPOINT_OPEN_PRESENCE as u64).collect());
        }
        assert!(WorldCore::restore("revoked-presence", limits, oversized_checkpoint).is_err());
    }

    #[test]
    fn admin_history_system_tail_after_trim_restores_as_partial() {
        let limits = WorldLimits {
            retained_events: 2,
            ..WorldLimits::default()
        };
        let mut core = WorldCore::new("partial-history", limits.clone()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        core.commit_prepared(WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        })
        .unwrap();
        for event in ["tick", "schedule", "presence"] {
            core.record_admin_history(WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: format!(r#"{{"cap":"host","event":"{event}"}}"#),
                },
            });
        }
        assert!(!core.admin_history_complete);
        assert!(
            core.admin_history
                .iter()
                .all(|event| matches!(&event.kind, WorldEventKind::System { .. }))
        );

        let checkpoint = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        let restored = WorldCore::restore("partial-history", limits, checkpoint).unwrap();
        assert_eq!(restored.core.sequence, 1);
        assert!(!restored.core.admin_history_complete);
        assert_eq!(restored.core.admin_history.len(), 2);
    }

    #[test]
    fn admin_history_byte_ledger_counts_unserializable_as_max_until_trimmed() {
        // Deliberate serialization-failure semantics (#4633): an event that
        // cannot be serialized counts as usize::MAX bytes — exactly as the
        // old rescan-every-time helper treated it — so it forces the
        // retained history to trim rather than occupying unaccounted space.
        let mut ledger = AdminHistoryByteLedger::default();
        ledger.track(Some(10));
        ledger.track(Some(20));
        assert_eq!(ledger.effective_bytes(), 30);
        assert_eq!(ledger.tracked(), 2);
        ledger.track(None);
        assert_eq!(ledger.effective_bytes(), usize::MAX);
        ledger.untrack_front();
        ledger.untrack_front();
        assert_eq!(
            ledger.effective_bytes(),
            usize::MAX,
            "the unserializable event is still retained"
        );
        ledger.untrack_front();
        assert_eq!(ledger.effective_bytes(), 0);
        assert_eq!(ledger.tracked(), 0);
    }

    #[test]
    fn admin_history_byte_ledger_stays_in_sync_through_count_trim() {
        let limits = WorldLimits {
            retained_events: 2,
            ..WorldLimits::default()
        };
        let mut core = WorldCore::new("ledger-trim-sync", limits).unwrap();
        for n in 0..5 {
            core.record_admin_history(WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: format!(r#"{{"cap":"host","event":"tick-{n}"}}"#),
                },
            });
        }
        assert_eq!(core.admin_history.len(), 2);
        assert!(!core.admin_history_complete);
        assert_eq!(
            core.admin_history_ledger.tracked(),
            core.admin_history.len()
        );
        let rescanned: usize = core
            .admin_history
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap().len())
            .sum();
        assert_eq!(core.admin_history_ledger.effective_bytes(), rescanned);
    }

    #[test]
    fn admin_history_byte_cap_trims_even_a_single_oversized_event() {
        let mut core = WorldCore::new("ledger-byte-cap", WorldLimits::default()).unwrap();
        core.record_admin_history(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: "small".to_owned(),
            },
        });
        assert_eq!(core.admin_history.len(), 1);
        core.record_admin_history(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: "x".repeat(MAX_ADMIN_HISTORY_BYTES + 1),
            },
        });
        assert!(
            core.admin_history.is_empty(),
            "an event larger than the byte cap cannot be retained"
        );
        assert!(!core.admin_history_complete);
        assert_eq!(core.admin_history_ledger.tracked(), 0);
        assert_eq!(core.admin_history_ledger.effective_bytes(), 0);
    }

    #[test]
    fn admin_history_byte_ledger_stays_in_sync_through_restore() {
        let mut core = WorldCore::new("ledger-restore-sync", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        core.commit_prepared(WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        })
        .unwrap();
        for event in ["tick", "schedule"] {
            core.record_admin_history(WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: format!(r#"{{"cap":"host","event":"{event}"}}"#),
                },
            });
        }
        let checkpoint = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        let restored =
            WorldCore::restore("ledger-restore-sync", WorldLimits::default(), checkpoint).unwrap();
        assert!(restored.core.admin_history_complete);
        assert_eq!(
            restored.core.admin_history_ledger.tracked(),
            restored.core.admin_history.len()
        );
        let rescanned: usize = restored
            .core
            .admin_history
            .iter()
            .map(|event| serde_json::to_vec(event).unwrap().len())
            .sum();
        assert_eq!(
            restored.core.admin_history_ledger.effective_bytes(),
            rescanned
        );
    }

    #[test]
    fn restore_rejects_checkpoint_admin_history_over_the_byte_bound() {
        let mut core = WorldCore::new("ledger-restore-bytes", WorldLimits::default()).unwrap();
        core.active_module_hash = Some("h".to_owned());
        core.commit_prepared(WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "h".to_owned(),
                seed: 7,
                provenance: None,
                trust: None,
            },
        })
        .unwrap();
        let mut checkpoint = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        if let Some(JournalEntry::Checkpoint { admin_history, .. }) = checkpoint.last_mut() {
            *admin_history = Some(JournalAdminHistory {
                events: vec![WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: "x".repeat(MAX_ADMIN_HISTORY_BYTES + 1),
                    },
                }],
                complete: false,
            });
        } else {
            panic!("checkpoint entry missing");
        }
        assert!(
            WorldCore::restore("ledger-restore-bytes", WorldLimits::default(), checkpoint).is_err(),
            "a checkpoint whose admin history exceeds the byte bound must be refused"
        );
    }

    #[tokio::test]
    async fn the_actor_journals_host_events_and_grants() {
        #[derive(Default, Clone)]
        struct Capturing(Arc<std::sync::Mutex<Vec<JournalEntry>>>);
        impl WorldJournal for Capturing {
            fn append(&mut self, entry: &JournalEntry) -> Result<()> {
                self.0.lock().unwrap().push(entry.clone());
                Ok(())
            }
        }
        let captured = Capturing::default();
        let (world, token) = {
            let core = WorldCore::new("j", WorldLimits::default()).unwrap();
            let handle = WorldHandle::spawn_durable(
                core,
                Box::new(Mock::new()),
                Some(Box::new(captured.clone())),
            );
            let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
            (handle, token)
        };
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"3".to_vec()).await.unwrap();
        // Wait deterministically for the host's chat line to be journaled
        // instead of settling for a fixed delay: it is the last artifact
        // this test asserts on, and the actor journals it before the
        // shutdown below can be observed.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let journaled = {
                let entries = captured.0.lock().unwrap();
                entries.iter().any(|entry| {
                    matches!(entry, JournalEntry::Committed { event } if matches!(&event.kind, WorldEventKind::Chat(text) if text == "hello from the program"))
                })
            };
            if journaled {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the host chat line was never journaled"
            );
            tokio::task::yield_now().await;
        }
        world.shutdown().await.unwrap();
        let entries = captured.0.lock().unwrap().clone();
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::CapsGranted { granted } if granted.contains(&"chat.say".to_owned()))),
            "{entries:?}"
        );
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::System { event } if event.kind == WorldEventKind::System { event: r#"{"cap":"result","ok":true,"value":{"sequence":2}}"#.to_owned() }))
                || entries
                    .iter()
                    .any(|entry| matches!(entry, JournalEntry::System { .. })),
            "the chat.say result is journaled: {entries:?}"
        );
        assert!(
            entries.iter().any(
                |entry| matches!(entry, JournalEntry::Committed { event } if matches!(&event.kind, WorldEventKind::Chat(text) if text == "hello from the program"))
            ),
            "the host chat line is journaled: {entries:?}"
        );
    }

    /// A program that only counts `time.tick` deliveries and publishes the count.
    #[derive(Debug, Default)]
    struct TickCounter {
        ticks: u64,
    }

    impl WorldProgram for TickCounter {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                let value: serde_json::Value =
                    serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                if value["cap"] == "time.tick" {
                    self.ticks += 1;
                }
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"v": 1, "subscribe": ["time.tick:20"]}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"ticks": self.ticks}).to_string()))
        }
    }

    /// The number of ticks a [`TickCounter`] actor has delivered, via its records.
    fn tick_count(actor: &WorldActor) -> u64 {
        actor
            .records_sender
            .borrow()
            .1
            .get("ticks")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    }

    /// A program that subscribes to fixed tick intervals and counts deliveries.
    #[derive(Debug)]
    struct FixedTicks {
        subscribe: Vec<String>,
        ticks: u64,
    }

    impl WorldProgram for FixedTicks {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                let value: serde_json::Value =
                    serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                if value["cap"] == "time.tick" {
                    self.ticks += 1;
                }
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"v": 1, "subscribe": self.subscribe.clone()}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"ticks": self.ticks}).to_string()))
        }
    }

    #[tokio::test]
    async fn sub_floor_ticks_are_refused_at_the_default_and_custom_floors() {
        for (floor, below, at) in [(1000, 999, 1000), (250, 249, 250)] {
            let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
            let limits = WorldLimits {
                min_tick_ms: floor,
                ..WorldLimits::default()
            };
            let mut core = WorldCore::new_with_clock("tick-floor", limits, clock.clone()).unwrap();
            core.ledger
                .set_granted(BTreeSet::from(["time.tick".to_owned()]));
            let (_, _token) = core
                .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
                .unwrap();
            let program = FixedTicks {
                subscribe: vec![format!("time.tick:{below}"), format!("time.tick:{at}")],
                ticks: 0,
            };
            let mut actor = WorldActor::for_test(core, Box::new(program), None);

            actor.after_change().await;
            assert!(
                actor.denied_subs.contains(&format!("time.tick:{below}")),
                "the sub-floor tick was not refused at floor {floor}"
            );

            actor
                .handle(WorldCommand::Presence {
                    participant: 1,
                    joined: true,
                })
                .await;
            assert!(!actor.ticks.contains_key(&below), "sub-floor tick armed");
            assert!(actor.ticks.contains_key(&at), "at-floor tick did not arm");

            clock.advance(at);
            actor.fire_ticks().await;
            assert_eq!(tick_count(&actor), 1, "only the at-floor tick delivered");
        }
    }

    /// A program that subscribes to `loc.watch:5000` and records every
    /// system event it is delivered, verbatim (#2341 slice 2).
    #[derive(Debug, Default)]
    struct LocWatchRecorder {
        events: Vec<String>,
    }

    impl WorldProgram for LocWatchRecorder {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.events.push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"v": 1, "subscribe": ["loc.watch:5000"]}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"events": self.events}).to_string()))
        }
    }

    /// The system events a [`LocWatchRecorder`] actor has delivered, via its records.
    fn loc_events(actor: &WorldActor) -> Vec<String> {
        actor
            .records_sender
            .borrow()
            .1
            .get("events")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn loc_watch_delivers_readings_and_revokes_on_consent_withdrawal() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core =
            WorldCore::new_with_clock("loc-watch", WorldLimits::default(), clock.clone()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["loc.watch".to_owned()]));
        let mut allowed = crate::world_caps::default_ceiling_names();
        allowed.insert("loc.watch".to_owned());
        core.ledger.set_allowed(allowed);
        let (_, _token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(LocWatchRecorder::default()), None);
        let reading = crate::world_caps::LocationReading {
            lat: 52.5,
            lon: 13.4,
            accuracy_m: 12.0,
            fix_age_ms: 250,
            source: crate::world_caps::LocationSource::Gnss,
        };
        actor.location_consent = crate::world_caps::LocationConsent::Granted;
        actor.set_location_fix(reading);

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        assert!(
            actor.loc_watches.contains_key(&5000),
            "a granted, consented watch arms on presence"
        );

        clock.advance(5000);
        actor.fire_loc_watches().await;
        // The fix was received at t=0 with fix_age_ms 250; at the
        // t=5000 delivery its true age is 5250 (Codex round-2 P2:
        // the age is updated at serve time, never served stale).
        let mut aged = reading;
        aged.fix_age_ms += 5000;
        assert_eq!(
            loc_events(&actor),
            vec![crate::world_caps::loc_watch_reading_event(&aged)],
            "the open watch delivers the host reading at its true age"
        );

        // Consent withdrawn: the next fire closes the watch with
        // exactly the revocation event, and nothing is delivered after.
        actor.location_consent = crate::world_caps::LocationConsent::ConsentRequired;
        clock.advance(5000);
        actor.fire_loc_watches().await;
        assert_eq!(
            loc_events(&actor),
            vec![
                crate::world_caps::loc_watch_reading_event(&aged),
                crate::world_caps::loc_watch_revocation_event(),
            ]
        );
        assert!(actor.loc_watches.is_empty(), "the revoked watch disarms");
        assert!(
            !actor
                .active_subs
                .contains(&crate::world_caps::Subscription::LocWatch(5000)),
            "the revoked watch is closed, not merely muted"
        );
    }

    #[tokio::test]
    async fn loc_watch_disarms_when_capability_authority_is_removed() {
        // Oracle: PR #8643 Codex P1 — revoking `loc.watch` authority
        // (here via SetCapsCeiling) stops location delivery: the
        // armed watch is disarmed by reconcile, and no reading is
        // delivered afterwards even though consent still stands.
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core =
            WorldCore::new_with_clock("loc-watch-revoke", WorldLimits::default(), clock.clone())
                .unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["loc.watch".to_owned()]));
        let mut allowed = crate::world_caps::default_ceiling_names();
        allowed.insert("loc.watch".to_owned());
        core.ledger.set_allowed(allowed);
        let (_, _token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(LocWatchRecorder::default()), None);
        let reading = crate::world_caps::LocationReading {
            lat: 52.5,
            lon: 13.4,
            accuracy_m: 12.0,
            fix_age_ms: 250,
            source: crate::world_caps::LocationSource::Gnss,
        };
        actor.location_consent = crate::world_caps::LocationConsent::Granted;
        actor.set_location_fix(reading);

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        assert!(
            actor.loc_watches.contains_key(&5000),
            "a granted, consented watch arms on presence"
        );
        clock.advance(5000);
        actor.fire_loc_watches().await;
        let mut aged = reading;
        aged.fix_age_ms += 5000;
        assert_eq!(
            loc_events(&actor),
            vec![crate::world_caps::loc_watch_reading_event(&aged)],
            "the open watch delivers before authority is removed"
        );

        // Remove loc.watch from the ceiling: authority is gone.
        let ceiling: Vec<String> = crate::world_caps::default_ceiling_names()
            .into_iter()
            .collect();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling { ceiling, reply })
            .await;
        let _ = response.await.unwrap().unwrap();
        assert!(
            actor.loc_watches.is_empty(),
            "the watch disarms when its capability authority is removed"
        );

        clock.advance(5000);
        actor.fire_loc_watches().await;
        assert_eq!(
            loc_events(&actor),
            vec![crate::world_caps::loc_watch_reading_event(&aged)],
            "no reading is delivered after authority removal"
        );
    }

    #[test]
    fn parse_wants_refuses_more_than_max_loc_watches() {
        // Oracle: PR #8643 Codex P2 — the advertised MAX_LOC_WATCHES
        // bound is enforced at wants parse time, like the tick bound.
        let over: Vec<String> = (0..=crate::world_caps::MAX_LOC_WATCHES)
            .map(|i| format!("loc.watch:{}", 5000 + i * 1000))
            .collect();
        let text = serde_json::json!({"v": 1, "subscribe": over}).to_string();
        assert!(
            crate::world_caps::parse_wants(&text).is_err(),
            "more than MAX_LOC_WATCHES location watches must not parse"
        );
        let at: Vec<String> = (0..crate::world_caps::MAX_LOC_WATCHES)
            .map(|i| format!("loc.watch:{}", 5000 + i * 1000))
            .collect();
        let text = serde_json::json!({"v": 1, "subscribe": at}).to_string();
        assert!(
            crate::world_caps::parse_wants(&text).is_ok(),
            "exactly MAX_LOC_WATCHES location watches must parse"
        );
    }

    /// A program whose loc.watch wants the test rewrites between
    /// `after_change` passes, via the shared cell (#8643 round-2 P2).
    #[derive(Debug)]
    struct VarLocProgram {
        wants: std::sync::Arc<std::sync::Mutex<String>>,
        events: Vec<String>,
    }

    impl WorldProgram for VarLocProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.events.push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(self.wants.lock().unwrap().clone()))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"events": self.events}).to_string()))
        }
    }

    #[tokio::test]
    async fn loc_watch_timers_are_pruned_when_subscriptions_leave_wants() {
        // Oracle: PR #8643 Codex round-2 P2 — when a program replaces
        // its requested watch interval, the old timer is disarmed by
        // the wants reconciliation itself, not left in loc_watches
        // until its distant deadline: the timer map must never hold
        // an interval that is no longer an active subscription.
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core =
            WorldCore::new_with_clock("loc-watch-prune", WorldLimits::default(), clock.clone())
                .unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["loc.watch".to_owned()]));
        let mut allowed = crate::world_caps::default_ceiling_names();
        allowed.insert("loc.watch".to_owned());
        core.ledger.set_allowed(allowed);
        let (_, _token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let wants = std::sync::Arc::new(std::sync::Mutex::new(
            serde_json::json!({"v": 1, "subscribe": ["loc.watch:60000"]}).to_string(),
        ));
        let mut actor = WorldActor::for_test(
            core,
            Box::new(VarLocProgram {
                wants: wants.clone(),
                events: Vec::new(),
            }),
            None,
        );
        actor.location_consent = crate::world_caps::LocationConsent::Granted;

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        assert!(
            actor.loc_watches.contains_key(&60000),
            "the requested watch arms"
        );

        // The program replaces its watch with a different interval:
        // the 60s timer must be pruned by this reconciliation, and
        // only the new interval stays armed.
        *wants.lock().unwrap() =
            serde_json::json!({"v": 1, "subscribe": ["loc.watch:5000"]}).to_string();
        actor.after_change().await;
        assert!(
            !actor.loc_watches.contains_key(&60000),
            "the replaced watch's timer is pruned when it leaves wants"
        );
        assert!(
            actor.loc_watches.contains_key(&5000),
            "the replacement watch arms"
        );
        assert_eq!(actor.loc_watches.len(), 1, "no stale timers accumulate");
    }

    #[tokio::test]
    async fn subscribed_ticks_arrive_only_while_someone_is_present() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core = WorldCore::new_with_clock(
            "ticks",
            WorldLimits {
                min_tick_ms: 20,
                ..WorldLimits::default()
            },
            clock.clone(),
        )
        .unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.tick".to_owned()]));
        let (_, _token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(TickCounter::default()), None);

        // The subscription is known but not armed while nobody is present.
        actor.after_change().await;
        assert!(actor.ticks.is_empty(), "no tick while nobody is present");

        // A presence transition arms the tick.
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        assert!(actor.ticks.contains_key(&20), "the tick armed on presence");

        clock.advance(20);
        actor.fire_ticks().await;
        assert_eq!(tick_count(&actor), 1, "the tick fired while present");

        // The last participant leaving disarms the tick for good.
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: false,
            })
            .await;
        assert!(actor.ticks.is_empty(), "the tick disarmed on leave");
        clock.advance(20);
        actor.fire_ticks().await;
        assert_eq!(
            tick_count(&actor),
            1,
            "ticks kept firing with nobody present"
        );
    }

    #[tokio::test]
    async fn ticks_follow_the_world_grant_not_a_participant_capability() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core = WorldCore::new_with_clock(
            "ticks",
            WorldLimits {
                min_tick_ms: 20,
                ..WorldLimits::default()
            },
            clock.clone(),
        )
        .unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.tick".to_owned()]));
        let owner_scopes = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let (_, owner) = core
            .issue_capability("ann", owner_scopes, CapabilityBounds::default())
            .unwrap();
        let short_lived = CapabilityBounds {
            expires_at: Some(150),
            uses: None,
        };
        let (_, short) = core
            .attenuate(&owner, WorldScopes::GUEST, short_lived, "bo")
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(TickCounter::default()), None);

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        // The short-lived delegation expires before the tick fires.
        clock.advance(150);
        assert!(
            actor.core.chat(&short, "late").is_err(),
            "the delegation expired"
        );
        // The world's tick grant still drives the tick while the owner is
        // present: a tick is not tied to a participant's capability.
        actor.fire_ticks().await;
        assert_eq!(
            tick_count(&actor),
            1,
            "the tick follows the world grant, not a capability"
        );
    }

    #[derive(Debug, Default)]
    struct TickSay {
        ticks: u64,
    }

    impl WorldProgram for TickSay {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind
                && event.contains("time.tick")
            {
                self.ticks += 1;
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let mut document = serde_json::json!({"v": 1, "subscribe": ["time.tick:20"]});
            if self.ticks > 0 {
                document["requests"] = serde_json::json!([{
                    "id": format!("say-{}", self.ticks),
                    "cap": "chat.say",
                    "args": {"text": "tick"},
                }]);
            }
            Ok(Some(document.to_string()))
        }
    }

    #[tokio::test]
    async fn ticks_speak_for_the_world_not_for_a_present_participant() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core = WorldCore::new_with_clock(
            "tick-attribution",
            WorldLimits {
                min_tick_ms: 20,
                ..WorldLimits::default()
            },
            clock.clone(),
        )
        .unwrap();
        core.ledger.set_granted(BTreeSet::from([
            "time.tick".to_owned(),
            "chat.say".to_owned(),
        ]));
        let (spectator, _) = core
            .issue_capability(
                "spectator",
                WorldScopes::JOIN.union(WorldScopes::INPUT),
                CapabilityBounds::default(),
            )
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(TickSay::default()), None);

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: spectator.id,
                joined: true,
            })
            .await;
        clock.advance(20);
        actor.fire_ticks().await;
        // The tick's follow-up request runs on the next change, with no acting
        // participant, so the world's grant decides it — not the spectator's scopes.
        actor.after_change().await;
        let usage = actor.core.ledger.usage()["chat.say"];
        assert_eq!(usage.allowed, 1, "the tick spoke for the world");
        assert_eq!(usage.denied, 0, "a tick was refused for a participant");
    }

    /// A program whose wanted request id flips from `say-stale` to `say-fresh`
    /// once it hears the first result: one `after_change` run proves both
    /// halves of the restart rule, with every delivered result recorded in
    /// `heard` for the test to inspect.
    struct ReopenSay {
        heard: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl WorldProgram for ReopenSay {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.heard.lock().unwrap().push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let (id, text) = if self.heard.lock().unwrap().is_empty() {
                ("say-stale", "before-restart")
            } else {
                ("say-fresh", "after-restart")
            };
            Ok(Some(
                serde_json::json!({
                    "v": 1,
                    "requests": [{
                        "id": id,
                        "cap": "chat.say",
                        "args": {"text": text},
                    }],
                })
                .to_string(),
            ))
        }
    }

    /// A program that always wants the same `chat.say` request.
    struct FixedSay;

    impl WorldProgram for FixedSay {
        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({
                    "v": 1,
                    "requests": [{
                        "id": "say-open",
                        "cap": "chat.say",
                        "args": {"text": "at-open"},
                    }],
                })
                .to_string(),
            ))
        }
    }

    fn core_chat_texts(core: &WorldCore) -> Vec<String> {
        chat_texts(&core.events.iter().cloned().collect::<Vec<_>>())
    }

    /// Restart semantics (#839): requests the program already listed when a
    /// restored world opened are refused with `actor_denied` / `restarted`,
    /// never re-executed — `handled_requests` is memory-only, so the actor
    /// cannot tell done from never-run, and a repeat would run unattributed
    /// with world authority.
    #[tokio::test]
    async fn reopened_wants_are_refused_not_reexecuted() {
        let mut core = world(16);
        core.ledger
            .set_granted(BTreeSet::from(["chat.say".to_owned()]));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut actor = WorldActor::for_test(
            core,
            Box::new(ReopenSay {
                heard: heard.clone(),
            }),
            None,
        );
        // What `WorldHandle::spawn_restored` captures at a restored open: the
        // program's current wants predate the restart.
        actor.stale_wants_at_open = BTreeSet::from(["say-stale".to_owned()]);

        actor.after_change().await;

        // The pre-existing request never ran: its side effect is absent ...
        let texts = core_chat_texts(&actor.core);
        assert!(
            texts.iter().all(|text| !text.contains("before-restart")),
            "stale request re-executed: {texts:?}"
        );
        // ... while the request the program issued after the open did.
        assert!(
            texts.iter().any(|text| text.contains("after-restart")),
            "fresh request did not execute: {texts:?}"
        );
        // The refusal never reached the ledger: exactly one execution was
        // counted, the fresh one.
        assert_eq!(actor.core.ledger.usage()["chat.say"].allowed, 1);
        // The stale id is settled: it stays in `handled_requests`, so a later
        // pass with the same wants cannot execute it.
        assert!(actor.handled_requests.contains("say-stale"));
        // The program heard the refusal first, with the restart reason, then
        // the fresh result.
        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 2, "unexpected deliveries: {heard:?}");
        let refusal: serde_json::Value = serde_json::from_str(&heard[0]).unwrap();
        assert_eq!(refusal["cap"], "result");
        assert_eq!(refusal["id"], "say-stale");
        assert_eq!(refusal["ok"], false);
        assert_eq!(refusal["error"], "actor_denied");
        assert_eq!(refusal["reason"], "restarted");
        let fresh: serde_json::Value = serde_json::from_str(&heard[1]).unwrap();
        assert_eq!(fresh["id"], "say-fresh");
        assert_eq!(fresh["ok"], true);
    }

    /// A program that wants a `client.file.read` whose id flips from
    /// `read-stale` to `read-fresh` once it hears the first result.
    struct ReopenRead {
        heard: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl WorldProgram for ReopenRead {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.heard.lock().unwrap().push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let id = if self.heard.lock().unwrap().is_empty() {
                "read-stale"
            } else {
                "read-fresh"
            };
            Ok(Some(
                serde_json::json!({
                    "v": 1,
                    "requests": [{
                        "id": id,
                        "cap": "client.file.read",
                        "args": {"path": "notes.txt"},
                    }],
                })
                .to_string(),
            ))
        }
    }

    /// F13.7 step 6: the restart rule covers client providers too. A
    /// pre-open `client.file.read` want is refused `restarted` and
    /// never routed to a client — no local file is read unattributed
    /// after a reopen — and the refusal joins the provider audit
    /// trail (broker-level coverage:
    /// `restarted_refusal_is_recorded_in_the_audit_trail`). A fresh
    /// want in the same pass still reaches the broker.
    #[tokio::test]
    async fn reopened_client_provider_wants_are_refused_not_routed() {
        let mut core = world(16);
        core.ledger
            .set_allowed(BTreeSet::from(["client.file.read".to_owned()]));
        core.ledger
            .set_granted(BTreeSet::from(["client.file.read".to_owned()]));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut actor = WorldActor::for_test(
            core,
            Box::new(ReopenRead {
                heard: heard.clone(),
            }),
            None,
        );
        actor.stale_wants_at_open = BTreeSet::from(["read-stale".to_owned()]);

        actor.after_change().await;

        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 2, "unexpected deliveries: {heard:?}");
        let refusal: serde_json::Value = serde_json::from_str(&heard[0]).unwrap();
        assert_eq!(refusal["id"], "read-stale");
        assert_eq!(refusal["ok"], false);
        assert_eq!(refusal["error"], "actor_denied");
        assert_eq!(refusal["reason"], "restarted");
        // The fresh want reached the broker, which refused it
        // unattributed (this harness has no joined session) rather
        // than routing it to any client.
        let fresh: serde_json::Value = serde_json::from_str(&heard[1]).unwrap();
        assert_eq!(fresh["id"], "read-fresh");
        assert_eq!(fresh["ok"], false);
        assert_eq!(fresh["error"], "actor_denied");
        assert_eq!(fresh["reason"], "unattributed");
        assert_eq!(actor.provider_broker.pending_count(), 0);
        assert!(actor.handled_requests.contains("read-stale"));
    }

    /// The capture half of the restart rule: `spawn_restored` refuses the
    /// requests present in `wants` at open, while a fresh spawn executes the
    /// same wants.
    #[tokio::test]
    async fn spawn_restored_refuses_open_time_wants() {
        for restored in [false, true] {
            let mut core = world(16);
            core.ledger
                .set_granted(BTreeSet::from(["chat.say".to_owned()]));
            let world = if restored {
                WorldHandle::spawn_restored(core, Box::new(FixedSay), None, None)
            } else {
                WorldHandle::spawn_with_program(core, Box::new(FixedSay))
            };
            let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            let seen = loop {
                let snapshot = world.snapshot(token.clone()).await.unwrap();
                let seen = snapshot.events.iter().any(|event| {
                    matches!(&event.kind, WorldEventKind::Chat(text) if text.contains("at-open"))
                });
                if seen || Instant::now() >= deadline {
                    break seen;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            };
            let usage = world.caps_report().await.unwrap().usage;
            if restored {
                assert!(!seen, "restored world re-executed its open-time wants");
                assert!(
                    usage.get("chat.say").is_none_or(|use_| use_.allowed == 0),
                    "restored world executed its open-time wants: {usage:?}"
                );
            } else {
                assert!(seen, "fresh world did not execute its wants");
            }
            world.shutdown().await.unwrap();
        }
    }

    #[test]
    fn stale_wants_capture_reads_the_current_wants() {
        let mut program = FixedSay;
        assert_eq!(
            WorldHandle::stale_wants_from(&mut program),
            BTreeSet::from(["say-open".to_owned()])
        );
    }

    #[tokio::test]
    async fn revoking_a_grant_applies_to_capabilities_already_issued() {
        let core = WorldCore::new("ceiling", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world.input(token.clone(), b"3".to_vec()).await.unwrap();
        let usage = world.caps_report().await.unwrap().usage["chat.say"];
        assert_eq!((usage.allowed, usage.denied), (1, 0));
        world
            .update_caps(Vec::new(), vec!["chat.say".to_owned()])
            .await
            .unwrap();
        world.input(token, b"3".to_vec()).await.unwrap();
        let usage = world.caps_report().await.unwrap().usage["chat.say"];
        assert_eq!((usage.allowed, usage.denied), (1, 1));
        world.shutdown().await.unwrap();
    }

    #[test]
    fn a_caps_ceiling_replays_from_the_journal() {
        let restored = WorldCore::restore(
            "ceil",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "ceil".to_owned(),
                    version: JOURNAL_VERSION,
                },
                JournalEntry::CapsGranted {
                    granted: vec!["time.now".to_owned(), "chat.say".to_owned()],
                },
                JournalEntry::CapsCeiling {
                    ceiling: vec!["time.now".to_owned()],
                },
            ],
        )
        .unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
        assert!(!restored.core.ledger.is_granted("chat.say"));
        assert!(
            restored.core.ledger.granted().contains("chat.say"),
            "the grant is kept"
        );

        let refused = WorldCore::restore(
            "ceil",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "ceil".to_owned(),
                    version: JOURNAL_VERSION,
                },
                JournalEntry::CapsCeiling {
                    ceiling: vec!["*".to_owned()],
                },
            ],
        );
        assert!(refused.is_err(), "a journaled ceiling may not use `*`");
    }

    #[test]
    fn a_caps_ceiling_survives_checkpoint_compaction_and_restore() {
        let mut core = world(8);
        core.caps_ceiling = BTreeSet::from(["time.now".to_owned()]);
        core.sync_ledger_ceiling();
        core.ledger
            .set_granted(BTreeSet::from(["chat.say".to_owned()]));
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Checkpoint { caps_ceiling: Some(ceiling), .. }
                if ceiling == &vec!["time.now".to_owned()]
        )));
        // The entry is what makes an older binary fail loudly on this journal.
        assert!(entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::CapsCeiling { ceiling } if ceiling == &vec!["time.now".to_owned()]
        )));
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.core.ledger.allowed(),
            &BTreeSet::from(["time.now".to_owned()])
        );
        assert!(!restored.core.ledger.is_granted("chat.say"));
    }

    #[test]
    fn a_default_ceiling_is_not_written_and_old_journals_allow_the_default_ceiling() {
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(entries.iter().all(|entry| !matches!(
            entry,
            JournalEntry::Checkpoint {
                caps_ceiling: Some(_),
                ..
            }
        )));
        let old = serde_json::json!({
            "entry": "checkpoint",
            "sequence": 0,
            "module_hash": "h",
            "seed": 1,
            "snapshot": "s",
            "events": []
        });
        let restored = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            [
                JournalEntry::Created {
                    world_id: "test-world".to_owned(),
                    version: JOURNAL_VERSION,
                },
                serde_json::from_value(old).unwrap(),
            ],
        )
        .unwrap();
        // An old journal with no written ceiling restores to the
        // current default ceiling: the whole catalog minus the sensor
        // caps (#2341), which join only an explicit ceiling.
        assert_eq!(
            restored.core.ledger.allowed(),
            &crate::world_caps::default_ceiling_names()
        );
    }

    #[test]
    fn the_server_ceiling_is_runtime_config_and_bounds_a_restored_grant() {
        let entries = [
            JournalEntry::Created {
                world_id: "server".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::CapsGranted {
                granted: vec!["time.now".to_owned(), "chat.say".to_owned()],
            },
        ];
        let lower = WorldLimits {
            server_caps: BTreeSet::from(["time.now".to_owned()]),
            ..WorldLimits::default()
        };
        let restored = WorldCore::restore("server", lower, entries.clone()).unwrap();
        assert!(restored.core.ledger.is_granted("time.now"));
        assert!(!restored.core.ledger.is_granted("chat.say"));
        assert!(restored.core.ledger.granted().contains("chat.say"));

        // Nothing about the server ceiling was journaled, so the default
        // server restores the grant.
        let restored = WorldCore::restore("server", WorldLimits::default(), entries).unwrap();
        assert!(restored.core.ledger.is_granted("chat.say"));
    }

    #[test]
    fn a_server_ceiling_naming_a_wildcard_is_refused() {
        let limits = WorldLimits {
            server_caps: BTreeSet::from(["*".to_owned()]),
            ..WorldLimits::default()
        };
        assert!(WorldCore::new("w", limits).is_err());
    }

    #[tokio::test]
    async fn a_ceiling_change_denies_a_use_that_a_prior_grant_allowed() {
        let core = WorldCore::new("ceiling", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        world
            .update_caps(vec!["chat.say".to_owned()], Vec::new())
            .await
            .unwrap();
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world.input(token.clone(), b"3".to_vec()).await.unwrap();
        let usage = world.caps_report().await.unwrap().usage["chat.say"];
        assert_eq!((usage.allowed, usage.denied), (1, 0));

        let report = world
            .set_caps_ceiling(vec!["time.now".to_owned()])
            .await
            .unwrap();
        assert_eq!(report.ceiling, vec!["time.now".to_owned()]);
        assert!(
            report.granted.contains(&"chat.say".to_owned()),
            "the grant is kept"
        );
        assert!(!report.effective.contains(&"chat.say".to_owned()));
        world.input(token, b"3".to_vec()).await.unwrap();
        let usage = world.caps_report().await.unwrap().usage["chat.say"];
        assert_eq!((usage.allowed, usage.denied), (1, 1));

        assert!(world.set_caps_ceiling(vec!["*".to_owned()]).await.is_err());
        assert!(
            world
                .set_caps_ceiling(vec!["time.nwo".to_owned()])
                .await
                .is_err()
        );
        world.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn lowering_the_ceiling_stops_an_active_subscription_and_reports_it_refused() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(0));
        let mut core = WorldCore::new_with_clock(
            "ceiling-ticks",
            WorldLimits {
                min_tick_ms: 20,
                ..WorldLimits::default()
            },
            clock.clone(),
        )
        .unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.tick".to_owned()]));
        let (_, _token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(TickCounter::default()), None);

        actor.after_change().await;
        actor
            .handle(WorldCommand::Presence {
                participant: 1,
                joined: true,
            })
            .await;
        clock.advance(20);
        actor.fire_ticks().await;
        assert_eq!(
            tick_count(&actor),
            1,
            "the tick arrived while the ceiling allowed it"
        );

        // Lowering the ceiling drops the subscription and reports it refused.
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling {
                ceiling: vec!["time.now".to_owned()],
                reply,
            })
            .await;
        let _ = response.await.unwrap().unwrap();
        actor.after_change().await;
        assert!(actor.core.ledger.usage()["time.tick"].denied > 0);
        assert!(!actor.core.ledger.effective().contains("time.tick"));
        // No tick follows while the ceiling excludes it.
        clock.advance(20);
        actor.fire_ticks().await;
        assert_eq!(
            tick_count(&actor),
            1,
            "ticks kept firing after the ceiling dropped them"
        );

        // Lifting the ceiling admits the refused subscription again.
        let full: Vec<String> = crate::world_caps::catalog_names().into_iter().collect();
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling {
                ceiling: full,
                reply,
            })
            .await;
        let _ = response.await.unwrap().unwrap();
        actor.after_change().await;
        clock.advance(20);
        actor.fire_ticks().await;
        assert_eq!(
            tick_count(&actor),
            2,
            "ticks did not resume once the ceiling allowed them"
        );
    }

    #[test]
    fn capability_scopes_isolate_actions() {
        let mut core = world(8);
        let (_, token) = join(&mut core, "Guest");
        assert!(core.snapshot(&token).is_ok());
        assert!(core.chat(&token, "hello").is_ok());
        assert!(core.input(&token, b"move").is_ok());
        let (_, join_only) = core
            .issue_capability("spectator", WorldScopes::JOIN, CapabilityBounds::default())
            .unwrap();
        assert!(core.snapshot(&join_only).is_ok());
        assert!(core.chat(&join_only, "no").is_err());
        assert!(core.input(&join_only, b"no").is_err());
    }

    #[test]
    fn chat_is_rate_limited_per_participant_within_a_ten_second_window() {
        let mut core = world(64);
        let (_, token) = join(&mut core, "Guest");
        for index in 0..CHAT_RATE_BURST {
            core.chat(&token, &format!("message {index}")).unwrap();
        }
        assert!(core.chat(&token, "one too many").is_err());
        assert_eq!(core.current_sequence(), u64::from(CHAT_RATE_BURST));
    }

    #[test]
    fn chat_refill_uses_monotonic_time_not_wall_clock_corrections() {
        let clock = Arc::new(SplitClock {
            wall_ms: AtomicU64::new(100_000),
            monotonic_ms: AtomicU64::new(0),
            monotonic_origin: Instant::now(),
        });
        let mut core =
            WorldCore::new_with_clock("test-world", WorldLimits::default(), clock.clone()).unwrap();
        let (_, token) = join(&mut core, "Guest");
        for index in 0..CHAT_RATE_BURST {
            core.chat(&token, &format!("message {index}")).unwrap();
        }
        assert!(core.chat(&token, "exhausted").is_err());

        // A forward wall-clock correction does not refill the bucket.
        clock.wall_ms.store(10_000_000, Ordering::SeqCst);
        assert!(core.chat(&token, "still exhausted").is_err());

        // A backward wall-clock correction cannot prevent a real ten-second
        // monotonic interval from refilling it.
        clock.wall_ms.store(1, Ordering::SeqCst);
        clock.monotonic_ms.store(10_000, Ordering::SeqCst);
        assert!(core.chat(&token, "refilled").is_ok());
    }

    fn in_an_hour() -> u64 {
        unix_ms() + 3_600_000
    }

    #[test]
    fn an_expired_capability_is_refused_for_every_operation() {
        let mut core = world(8);
        let (_, token) = core
            .issue_capability(
                "Ada",
                WorldScopes::GUEST,
                CapabilityBounds {
                    expires_at: Some(1),
                    uses: None,
                },
            )
            .unwrap();
        assert!(core.snapshot(&token).is_err());
        assert!(core.chat(&token, "late").is_err());
        assert!(core.input(&token, b"late").is_err());
    }

    #[test]
    fn delegation_may_not_outlive_or_expire_in_the_past() {
        let mut core = world(8);
        let (_, owner) = core
            .issue_capability(
                "Ada",
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
                CapabilityBounds {
                    expires_at: Some(in_an_hour()),
                    uses: None,
                },
            )
            .unwrap();
        let later = CapabilityBounds {
            expires_at: Some(in_an_hour() + 1),
            uses: None,
        };
        assert!(
            core.attenuate(&owner, WorldScopes::JOIN, later, "Bo")
                .is_err()
        );
        let past = CapabilityBounds {
            expires_at: Some(1),
            uses: None,
        };
        assert!(
            core.attenuate(&owner, WorldScopes::JOIN, past, "Cy")
                .is_err()
        );
        let sooner = CapabilityBounds {
            expires_at: Some(in_an_hour() - 1_000),
            uses: None,
        };
        assert!(
            core.attenuate(&owner, WorldScopes::JOIN, sooner, "Di")
                .is_ok()
        );
        let inherited = core
            .attenuate(&owner, WorldScopes::JOIN, CapabilityBounds::default(), "Ed")
            .unwrap()
            .1;
        assert!(core.snapshot(&inherited).is_ok());
    }

    #[test]
    fn a_host_issued_limit_draws_down_through_every_delegation() {
        let mut core = world(8);
        let (_, owner) = core
            .issue_capability(
                "Ada",
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
                CapabilityBounds {
                    expires_at: None,
                    uses: Some(3),
                },
            )
            .unwrap();
        let (_, child) = core
            .attenuate(
                &owner,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        core.chat(&child, "one").unwrap();
        core.chat(&child, "two").unwrap();
        core.chat(&owner, "last").unwrap();
        assert!(
            core.chat(&child, "three").is_err(),
            "the host's limit bounds the delegate"
        );
        assert!(core.chat(&owner, "over").is_err());
        assert!(core.snapshot(&owner).is_ok(), "reading is not a use");
    }

    #[test]
    fn a_delegated_grant_carries_no_use_count() {
        let mut core = world(8);
        let delegator = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let (_, owner) = core
            .issue_capability("Ada", delegator, CapabilityBounds::default())
            .unwrap();
        let limited = CapabilityBounds {
            expires_at: None,
            uses: Some(1),
        };
        assert!(
            core.attenuate(&owner, WorldScopes::GUEST, limited, "Bo")
                .is_err(),
            "a delegation carries no use count"
        );
        let (child, child_token) = core
            .attenuate(
                &owner,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Cy",
            )
            .unwrap();
        for text in ["one", "two", "three"] {
            core.chat(&child_token, text).unwrap();
        }
        let record = serde_json::to_value(core.issued_entry(child.id).unwrap()).unwrap();
        assert!(
            record.get("uses").is_none(),
            "the grant record has no use count: {record}"
        );
    }

    #[test]
    fn a_grant_without_a_bound_is_never_refused_for_count() {
        let mut core = world(8);
        let (_, token) = join(&mut core, "Ada");
        for i in 0..50 {
            core.chat(&token, &format!("m{i}")).unwrap();
        }
        assert!(core.snapshot(&token).is_ok());
    }

    #[test]
    fn a_host_issued_limit_is_enforced_by_the_host_alone() {
        let mut core = world(8);
        let (_, token) = core
            .issue_capability(
                "Ada",
                WorldScopes::GUEST,
                CapabilityBounds {
                    expires_at: None,
                    uses: Some(2),
                },
            )
            .unwrap();
        core.chat(&token, "one").unwrap();
        core.chat(&token, "two").unwrap();
        assert!(core.chat(&token, "three").is_err());
        assert_eq!(core.current_sequence(), 2, "a refused use commits nothing");
    }

    #[test]
    fn use_counts_are_local_and_do_not_converge_across_partitions() {
        let mut source = world(8);
        let (owner, token) = source
            .issue_capability(
                "Ada",
                WorldScopes::GUEST,
                CapabilityBounds {
                    expires_at: None,
                    uses: Some(1),
                },
            )
            .unwrap();
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: owner.clone(),
                digest: hex_encode(&token_digest(token.expose())),
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: Some(1),
                used: 0,
                subject: None,
            },
        ];
        // Two partitions start from the same journal and do not see each other.
        let mut north = WorldCore::restore("test-world", WorldLimits::default(), entries.clone())
            .unwrap()
            .core;
        let mut south = WorldCore::restore("test-world", WorldLimits::default(), entries)
            .unwrap()
            .core;
        north.chat(&token, "north").unwrap();
        south.chat(&token, "south").unwrap();
        // Each side counted its own use, so the one-use grant was used twice.
        assert!(north.chat(&token, "north again").is_err());
        assert!(south.chat(&token, "south again").is_err());
        // Each state is still valid: sequence and tally agree, and the grant is held.
        for core in [&north, &south] {
            assert_eq!(core.current_sequence(), 1);
            assert_eq!(core.capabilities[&owner.id].used, 1);
            assert_eq!(core.participants().len(), 1);
        }
    }

    #[test]
    fn use_counts_survive_restore_and_compaction() {
        let mut source = world(8);
        let (owner, token) = source
            .issue_capability(
                "Ada",
                WorldScopes::GUEST,
                CapabilityBounds {
                    expires_at: None,
                    uses: Some(2),
                },
            )
            .unwrap();
        let digest = hex_encode(&token_digest(token.expose()));
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: owner,
                digest,
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: Some(2),
                used: 1,
                subject: None,
            },
        ];
        let mut restored =
            WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        restored.core.chat(&token, "second").unwrap();
        assert!(restored.core.chat(&token, "third").is_err());
    }

    #[test]
    fn restore_refuses_a_delegate_outliving_its_parent() {
        let mut source = world(8);
        let (owner, owner_token) = source
            .issue_capability(
                "Ada",
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
                CapabilityBounds {
                    expires_at: Some(in_an_hour()),
                    uses: None,
                },
            )
            .unwrap();
        let (child, child_token) = source
            .attenuate(
                &owner_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        let digest = hex_encode(&token_digest(child_token.expose()));
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: owner,
                digest: hex_encode(&token_digest(owner_token.expose())),
                scopes: WorldScopes::GUEST.union(WorldScopes::DELEGATE).bits(),
                parent: None,
                expires_at: Some(in_an_hour()),
                uses: None,
                used: 0,
                subject: None,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: child,
                digest,
                scopes: WorldScopes::JOIN.bits(),
                parent: Some(1),
                expires_at: Some(in_an_hour() + 1),
                uses: None,
                used: 0,
                subject: None,
            },
        ];
        assert!(WorldCore::restore("test-world", WorldLimits::default(), entries).is_err());
    }

    #[test]
    fn restore_does_not_resurrect_a_delegate_issued_after_parent_revocation() {
        let mut source = world(8);
        let (owner, owner_token) = source
            .issue_capability(
                "Ada",
                WorldScopes::GUEST.union(WorldScopes::DELEGATE),
                CapabilityBounds::default(),
            )
            .unwrap();
        let (child, child_token) = source
            .attenuate(
                &owner_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        let created = JournalEntry::Created {
            world_id: "test-world".to_owned(),
            version: MAX_JOURNAL_VERSION,
        };
        let owner_issued = source.issued_entry(owner.id).unwrap();
        let parent_revoked = JournalEntry::Revoked {
            revoked_at: None,
            participant_id: owner.id,
            by: None,
        };
        let child_issued = source.issued_entry(child.id).unwrap();

        // A hand-written journal can omit the child's redundant Revoked entry.
        // The earlier parent revocation must still keep the child unusable.
        let restored = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            vec![
                created.clone(),
                owner_issued.clone(),
                parent_revoked.clone(),
                child_issued.clone(),
            ],
        )
        .unwrap();
        assert!(
            restored
                .core
                .authorize(&child_token, WorldScopes::JOIN)
                .is_err()
        );

        // Compaction emits the descendant's own Issued/Revoked pair after its
        // already-revoked parent; that legitimate order must also restore.
        let compacted = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            vec![
                created,
                owner_issued,
                parent_revoked,
                child_issued,
                JournalEntry::Revoked {
                    revoked_at: None,
                    participant_id: child.id,
                    by: None,
                },
            ],
        )
        .unwrap();
        assert!(
            compacted
                .core
                .authorize(&child_token, WorldScopes::JOIN)
                .is_err()
        );
    }

    #[test]
    fn restore_refuses_an_issued_participant_zero() {
        // #2972: participant 0 is the reserved host/system identity —
        // mint can never issue it, so a journal issuing it to itself
        // (with a self-chosen digest) must be refused outright, whether
        // the forged entry stands alone or follows legitimate entries.
        let mut source = world(8);
        let (owner, owner_token) = source
            .issue_capability("Ada", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let legitimate = JournalEntry::Issued {
            issued_at: None,
            participant: owner,
            digest: hex_encode(&token_digest(owner_token.expose())),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        let forged = JournalEntry::Issued {
            issued_at: None,
            participant: Participant {
                id: 0,
                display_name: "host".to_owned(),
            },
            digest: "ab".repeat(32),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        let header = JournalEntry::Created {
            world_id: "test-world".to_owned(),
            version: JOURNAL_VERSION,
        };
        for entries in [
            vec![header.clone(), forged.clone()],
            vec![header, legitimate, forged],
        ] {
            let err = match WorldCore::restore("test-world", WorldLimits::default(), entries) {
                Ok(_) => panic!("restore must refuse an Issued participant 0"),
                Err(err) => err,
            };
            assert!(
                err.to_string().contains("reserved host identity"),
                "error must name the reserved id: {err}"
            );
        }
    }

    #[test]
    fn restore_refuses_issued_display_names_mint_could_not_produce() {
        // #2960: restore inserts journaled participants verbatim, but
        // mint sanitizes every display name (control/format characters
        // stripped, 48-char cap, trimmed) and refuses an empty result.
        // A journaled name mint could never have produced — empty,
        // control-bearing, over-length, or untrimmed — must refuse the
        // whole restore, while a legitimately minted journal restores
        // with byte-identical names.
        let mut source = world(8);
        let (owner, owner_token) = source
            .issue_capability("Ada", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let legitimate = JournalEntry::Issued {
            issued_at: None,
            participant: owner.clone(),
            digest: hex_encode(&token_digest(owner_token.expose())),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        let header = JournalEntry::Created {
            world_id: "test-world".to_owned(),
            version: JOURNAL_VERSION,
        };
        // A legitimately minted name restores fine.
        WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            [header.clone(), legitimate],
        )
        .unwrap();
        // A name minted before #2112's format stripping — a fixed point
        // of the legacy sanitizer only, here a ZWJ family emoji — also
        // restores fine: those journals predate the current sanitizer
        // and restored successfully before this check existed.
        let legacy = JournalEntry::Issued {
            issued_at: None,
            participant: Participant {
                id: 1,
                display_name: "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}".to_owned(),
            },
            digest: "ab".repeat(32),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            [header.clone(), legacy],
        )
        .unwrap();
        // A name minted before #4711 — 48 chars ending in a space, the
        // exact output of the pre-fix sanitizer for a longer input —
        // also restores fine: that journal was written by a live mint
        // path, and refusing it bricks the world.
        let boundary = JournalEntry::Issued {
            issued_at: None,
            participant: Participant {
                id: 1,
                display_name: format!("{} ", "a".repeat(47)),
            },
            digest: "ab".repeat(32),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            [header.clone(), boundary],
        )
        .unwrap();
        // And a name minted now from a boundary input (>48 chars,
        // whitespace at position 48) is trimmed at mint and restores.
        let (minted, minted_token) = source
            .issue_capability(
                &format!("{} {}", "a".repeat(47), "b".repeat(10)),
                WorldScopes::GUEST,
                CapabilityBounds::default(),
            )
            .unwrap();
        assert_eq!(minted.display_name, "a".repeat(47));
        let minted_entry = JournalEntry::Issued {
            issued_at: None,
            participant: minted.clone(),
            digest: hex_encode(&token_digest(minted_token.expose())),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
        };
        WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            [header.clone(), minted_entry],
        )
        .unwrap();
        for bad_name in [
            String::new(),
            "a\u{7}\n".to_owned() + &"x".repeat(98),
            "a".repeat(49),
            "Ada ".to_owned(),
            "\u{1}\u{2}".to_owned(),
        ] {
            let forged = JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id: 1,
                    display_name: bad_name.clone(),
                },
                digest: "ab".repeat(32),
                scopes: WorldScopes::GUEST.bits(),
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            };
            let err = match WorldCore::restore(
                "test-world",
                WorldLimits::default(),
                [header.clone(), forged],
            ) {
                Ok(_) => panic!("restore must refuse display name {bad_name:?}"),
                Err(err) => err,
            };
            assert!(
                err.to_string().contains("display name"),
                "error must name the display name rule: {err}"
            );
        }
    }

    #[test]
    fn restore_refuses_more_issued_participants_than_the_limit() {
        let mut source = world(8);
        let mut entries = vec![JournalEntry::Created {
            world_id: "test-world".to_owned(),
            version: MAX_JOURNAL_VERSION,
        }];
        for name in ["Ada", "Bo", "Cy"] {
            let (participant, _) = join(&mut source, name);
            entries.push(source.issued_entry(participant.id).unwrap());
        }

        let error = WorldCore::restore(
            "test-world",
            WorldLimits {
                participants: 2,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("participant 3") && error.to_string().contains("limit 2"),
            "error must name the rejected participant and limit: {error}"
        );
    }

    #[test]
    fn restore_accepts_issued_participants_at_the_limit() {
        let mut source = world(8);
        let (first, _) = join(&mut source, "Ada");
        let (second, _) = join(&mut source, "Bo");
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: MAX_JOURNAL_VERSION,
            },
            source.issued_entry(first.id).unwrap(),
            source.issued_entry(second.id).unwrap(),
        ];

        let restored = WorldCore::restore(
            "test-world",
            WorldLimits {
                participants: 2,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap();
        assert_eq!(restored.core.participants().len(), 2);
    }

    #[test]
    fn restore_releases_participant_capacity_after_revocation() {
        let mut source = world(8);
        let (first, _) = join(&mut source, "Ada");
        let (second, _) = join(&mut source, "Bo");
        assert_eq!(source.revoke_tree(first.id), vec![first.id]);
        let (third, _) = join(&mut source, "Cy");
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: MAX_JOURNAL_VERSION,
            },
            source.issued_entry(first.id).unwrap(),
            source.issued_entry(second.id).unwrap(),
            JournalEntry::Revoked {
                revoked_at: None,
                participant_id: first.id,
                by: None,
            },
            source.issued_entry(third.id).unwrap(),
        ];

        let restored = WorldCore::restore(
            "test-world",
            WorldLimits {
                participants: 2,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap();
        let mut active_ids: Vec<_> = restored
            .core
            .participants()
            .into_iter()
            .map(|participant| participant.id)
            .collect();
        active_ids.sort_unstable();
        assert_eq!(active_ids, vec![second.id, third.id]);
    }

    #[test]
    fn restore_accepts_compacted_journal_at_active_participant_limit() {
        let mut source = world(8);
        let (first, _) = join(&mut source, "Ada");
        source.revoke_tree(first.id);
        let (second, _) = join(&mut source, "Bo");
        let (third, _) = join(&mut source, "Cy");
        source.active_module_hash = Some("module-hash".to_owned());
        let entries = source.checkpoint_entries("snapshot".to_owned()).unwrap();

        let restored = WorldCore::restore(
            "test-world",
            WorldLimits {
                participants: 2,
                ..WorldLimits::default()
            },
            entries,
        )
        .unwrap();
        assert_eq!(restored.core.capabilities.len(), 3);
        let mut active_ids: Vec<_> = restored
            .core
            .participants()
            .into_iter()
            .map(|participant| participant.id)
            .collect();
        active_ids.sort_unstable();
        assert_eq!(active_ids, vec![second.id, third.id]);
    }

    #[test]
    fn events_are_sequenced_and_reconnect_can_catch_up() {
        let mut core = world(8);
        let (_, token) = join(&mut core, "Ada");
        assert_eq!(core.chat(&token, "hello").unwrap().sequence, 1);
        assert_eq!(core.input(&token, b"move").unwrap().sequence, 2);
        let page = core.events_after(&token, 1).unwrap();
        assert_eq!(page.current_sequence, 2);
        assert!(!page.needs_snapshot);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].sequence, 2);
    }

    #[test]
    fn reconnect_requires_snapshot_when_cursor_is_older_than_retention() {
        let mut core = world(2);
        let (_, token) = join(&mut core, "Ada");
        for msg in ["one", "two", "three"] {
            core.chat(&token, msg).unwrap();
        }
        let page = core.events_after(&token, 0).unwrap();
        assert!(page.needs_snapshot);
        assert!(page.events.is_empty());
        let snapshot = core.snapshot(&token).unwrap();
        assert_eq!(snapshot.current_sequence, 3);
        assert_eq!(snapshot.oldest_retained_sequence, 2);
        assert_eq!(snapshot.events.len(), 2);
    }

    /// #11778: a checkpoint keeps who revoked a capability. It wrote
    /// `by: None` for every revocation, so after compaction a moderator's
    /// revoke replayed as the admin's and the audit actor was lost.
    /// #11960: mint stamps the world clock's time on each capability,
    /// a revoke stamps every capability it revokes, and a checkpoint and a
    /// restore keep both, delegated descendants included.
    #[test]
    fn join_and_revoke_times_survive_checkpoint_and_restore_11960() {
        fn times(entries: &[JournalEntry]) -> Vec<(u64, Option<u64>, Option<u64>)> {
            let mut issued: Vec<(u64, Option<u64>, Option<u64>)> = entries
                .iter()
                .filter_map(|entry| match entry {
                    JournalEntry::Issued {
                        participant,
                        issued_at,
                        ..
                    } => Some((participant.id, *issued_at, None)),
                    _ => None,
                })
                .collect();
            for entry in entries {
                if let JournalEntry::Revoked {
                    participant_id,
                    revoked_at,
                    ..
                } = entry
                    && let Some(row) = issued.iter_mut().find(|row| row.0 == *participant_id)
                {
                    row.2 = *revoked_at;
                }
            }
            issued
        }
        let clock = Arc::new(SplitClock {
            wall_ms: AtomicU64::new(1_000),
            monotonic_ms: AtomicU64::new(0),
            monotonic_origin: Instant::now(),
        });
        let mut core =
            WorldCore::new_with_clock("test-world", WorldLimits::default(), clock.clone()).unwrap();
        core.active_module_hash = Some("module".to_owned());
        let root = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let (owner, owner_token) = core
            .issue_capability("Owner", root, CapabilityBounds::default())
            .unwrap();
        clock.wall_ms.store(2_000, Ordering::SeqCst);
        let (child, _) = core
            .delegate(
                &owner_token,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Child",
                None,
            )
            .unwrap();
        clock.wall_ms.store(3_000, Ordering::SeqCst);
        let (kept, _) = join(&mut core, "Kept");
        let mut revoked = core.revoke_tree_at(owner.id, None, Some(4_000));
        revoked.sort_unstable();
        assert_eq!(revoked, vec![owner.id, child.id]);
        // Revoking a revoked capability again changes no recorded time.
        assert!(core.revoke_tree_at(owner.id, None, Some(9_999)).is_empty());

        let expected = vec![
            (owner.id, Some(1_000), Some(4_000)),
            (child.id, Some(2_000), Some(4_000)),
            (kept.id, Some(3_000), None),
        ];
        let compacted = core.checkpoint_entries("{}".to_owned()).unwrap();
        assert_eq!(times(&compacted), expected);
        // The header names the version its timed entries require.
        assert!(matches!(
            compacted.first(),
            Some(JournalEntry::Created { version, .. }) if *version >= JOURNAL_VERSION_JOIN_TIMES
        ));
        let restored = WorldCore::restore("test-world", WorldLimits::default(), compacted)
            .unwrap()
            .core;
        assert_eq!(
            times(&restored.checkpoint_entries("{}".to_owned()).unwrap()),
            expected
        );
    }

    /// #11960: an entry carrying a time requires version 12, one without
    /// stays readable by older binaries, and a header older than the entry
    /// is refused at restore.
    #[test]
    fn timed_entries_require_the_join_times_version_11960() {
        let untimed = issued_entry_for(1, WorldScopes::GUEST.bits());
        assert_eq!(journal_version_required_by(&untimed), JOURNAL_VERSION);
        let JournalEntry::Issued {
            participant,
            digest,
            scopes,
            parent,
            expires_at,
            uses,
            used,
            subject,
            ..
        } = untimed
        else {
            unreachable!()
        };
        let timed = JournalEntry::Issued {
            participant,
            digest,
            scopes,
            parent,
            expires_at,
            uses,
            used,
            subject,
            issued_at: Some(5),
        };
        assert_eq!(
            journal_version_required_by(&timed),
            JOURNAL_VERSION_JOIN_TIMES
        );
        let revoke = |revoked_at| JournalEntry::Revoked {
            participant_id: 1,
            by: None,
            revoked_at,
        };
        assert_eq!(journal_version_required_by(&revoke(None)), JOURNAL_VERSION);
        assert_eq!(
            journal_version_required_by(&revoke(Some(6))),
            JOURNAL_VERSION_JOIN_TIMES
        );
        let header = |version| JournalEntry::Created {
            world_id: "test-world".to_owned(),
            version,
        };
        let old = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            vec![header(JOURNAL_VERSION_PROGRAM_BUNDLE), timed.clone()],
        );
        let error = old.err().expect("an older header refuses a timed entry");
        assert!(
            error.to_string().contains("requires version 12"),
            "{error:#}"
        );
        let new = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            vec![header(JOURNAL_VERSION_JOIN_TIMES), timed],
        )
        .unwrap()
        .core;
        assert_eq!(new.capabilities.get(&1).unwrap().issued_at, Some(5));
        // An untimed entry still reads, and its time stays unknown, not zero.
        let legacy = WorldCore::restore(
            "test-world",
            WorldLimits::default(),
            vec![
                header(JOURNAL_VERSION),
                issued_entry_for(1, WorldScopes::GUEST.bits()),
            ],
        )
        .unwrap()
        .core;
        assert_eq!(legacy.capabilities.get(&1).unwrap().issued_at, None);
    }

    #[test]
    fn checkpoint_keeps_the_revoking_moderator_11778() {
        fn revocations(entries: &[JournalEntry]) -> Vec<(u64, Option<u64>)> {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    JournalEntry::Revoked {
                        participant_id, by, ..
                    } => Some((*participant_id, *by)),
                    _ => None,
                })
                .collect()
        }
        let mut core = world(8);
        core.active_module_hash = Some("module".to_owned());
        let (moderated, _) = join(&mut core, "Ada");
        let (by_admin, _) = join(&mut core, "Bo");
        let (moderator, _) = join(&mut core, "Cy");
        assert_eq!(
            core.revoke_tree_by(moderated.id, Some(moderator.id)),
            vec![moderated.id]
        );
        assert_eq!(core.revoke_tree_by(by_admin.id, None), vec![by_admin.id]);
        // A second revoke of a revoked capability changes nothing, the
        // recorded actor included.
        assert!(
            core.revoke_tree_by(moderated.id, Some(by_admin.id))
                .is_empty()
        );

        let compacted = core.checkpoint_entries("{}".to_owned()).unwrap();
        let expected = vec![(moderated.id, Some(moderator.id)), (by_admin.id, None)];
        assert_eq!(revocations(&compacted), expected);

        // Replaying the checkpoint and compacting again keeps the actor.
        let mut restored = WorldCore::restore("test-world", WorldLimits::default(), compacted)
            .unwrap()
            .core;
        restored.active_module_hash = Some("module".to_owned());
        let again = restored.checkpoint_entries("{}".to_owned()).unwrap();
        assert_eq!(revocations(&again), expected);
    }

    #[test]
    fn revoked_capabilities_cannot_read_or_write() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "Ada");
        assert_eq!(core.revoke_tree(participant.id), vec![participant.id]);
        assert!(core.revoke_tree(participant.id).is_empty());
        assert!(core.snapshot(&token).is_err());
        assert!(core.chat(&token, "hello").is_err());
    }

    #[test]
    fn content_before_a_removal_is_kept_and_content_after_it_is_refused() {
        let mut core = world(8);
        let (ada, ada_token) = join(&mut core, "Ada");
        let (_, bo_token) = join(&mut core, "Bo");
        core.chat(&ada_token, "before").unwrap();
        assert_eq!(core.revoke_tree(ada.id), vec![ada.id]);
        assert!(core.chat(&ada_token, "after").is_err());
        assert!(core.input(&ada_token, b"move").is_err());
        assert_eq!(
            core.current_sequence(),
            1,
            "a refused action commits nothing"
        );
        let kept: Vec<String> = core
            .snapshot(&bo_token)
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.kind {
                WorldEventKind::Chat(text) => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(kept, vec!["before"]);
        assert_eq!(core.events_after(&bo_token, 0).unwrap().events.len(), 1);
    }

    #[tokio::test]
    async fn a_removal_racing_chat_keeps_every_accepted_line() {
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        let (ada, ada_token) = handle.issue("Ada", WorldScopes::GUEST).await.unwrap();
        let (_, bo_token) = handle.issue("Bo", WorldScopes::GUEST).await.unwrap();
        let chatter = {
            let handle = handle.clone();
            let token = ada_token.clone();
            tokio::spawn(async move {
                let mut accepted = 0;
                for i in 0..200 {
                    if handle.chat(token.clone(), format!("m{i}")).await.is_ok() {
                        accepted += 1;
                    }
                }
                accepted
            })
        };
        tokio::task::yield_now().await;
        assert!(handle.revoke(ada.id).await.unwrap());
        let accepted = chatter.await.unwrap();
        assert!(handle.chat(ada_token, "late").await.is_err());
        let snapshot = handle.snapshot(bo_token).await.unwrap();
        let from_ada = snapshot
            .events
            .iter()
            .filter(|event| event.participant_id == ada.id)
            .count();
        assert_eq!(
            from_ada, accepted,
            "every line accepted before the removal is kept, and none after it"
        );
    }

    /// #11775: metrics count distinct connected participants as well as
    /// sessions, so an owner report's active count is not inflated by one
    /// participant holding several sessions.
    #[tokio::test]
    async fn metrics_count_connected_participants_and_sessions() {
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        let (ada, _) = handle.issue("Ada", WorldScopes::GUEST).await.unwrap();
        let (bo, _) = handle.issue("Bo", WorldScopes::GUEST).await.unwrap();
        let idle = handle.metrics().await.unwrap();
        assert_eq!((idle.sessions, idle.participants), (0, 0));
        handle.presence(ada.id, true).await;
        handle.presence(ada.id, true).await;
        handle.presence(bo.id, true).await;
        let busy = handle.metrics().await.unwrap();
        assert_eq!((busy.sessions, busy.participants), (3, 2));
        handle.presence(bo.id, false).await;
        let after = handle.metrics().await.unwrap();
        assert_eq!((after.sessions, after.participants), (2, 1));
    }

    #[test]
    fn role_presets_grant_and_withhold_the_expected_effects() {
        let spectator = WorldScopes::from_role("spectator").unwrap();
        assert!(spectator.allows_effect("time.now"));
        assert!(!spectator.allows_effect("chat.say"));
        assert!(!spectator.allows_effect("chat.history"));
        assert!(!spectator.reads_chat());
        assert!(spectator.attenuate(WorldScopes::INPUT).is_none());

        // An observer reads chat, through the stream and `chat.history`
        // alike, and still cannot send it or act.
        let observer = WorldScopes::from_role("observer").unwrap();
        assert!(observer.reads_chat());
        assert!(observer.allows_effect("chat.history"));
        assert!(!observer.allows_effect("chat.say"));
        assert!(!observer.contains(WorldScopes::INPUT));

        let player = WorldScopes::from_role("player").unwrap();
        assert!(player.contains(WorldScopes::INPUT));
        assert!(!player.contains(WorldScopes::CHAT));
        assert!(!player.contains(WorldScopes::DELEGATE));

        let guest = WorldScopes::from_role("guest").unwrap();
        assert_eq!(guest, WorldScopes::GUEST);
        assert!(guest.allows_effect("chat.say"));

        let delegator = WorldScopes::from_role("delegator").unwrap();
        assert!(delegator.contains(WorldScopes::DELEGATE));
        assert!(delegator.attenuate(WorldScopes::JOIN).is_some());

        let builder = WorldScopes::from_role("builder").unwrap();
        assert!(builder.is_builder());
        let moderator = WorldScopes::from_role("moderator").unwrap();
        assert!(moderator.is_moderator());
        assert_eq!(
            WorldScopes::ROLES,
            &[
                "spectator",
                "observer",
                "player",
                "guest",
                "delegator",
                "builder",
                "moderator"
            ]
        );
    }

    #[test]
    fn attenuation_only_narrows_and_needs_delegate() {
        let join = WorldScopes::JOIN;
        let delegator = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        assert_eq!(delegator.attenuate(join), Some(join));
        assert!(delegator.attenuate(WorldScopes::GUEST).is_some());
        assert!(
            WorldScopes::GUEST.attenuate(join).is_none(),
            "no delegate scope"
        );
        assert!(
            delegator.attenuate(WorldScopes::CHAT).is_none(),
            "must keep join"
        );
        let join_only = WorldScopes::JOIN.union(WorldScopes::DELEGATE);
        assert!(
            join_only.attenuate(WorldScopes::GUEST).is_none(),
            "widening"
        );
    }

    #[test]
    fn scope_names_and_effects_follow_the_scopes() {
        assert_eq!(
            WorldScopes::from_names(["join", "delegate"]),
            WorldScopes::from_bits(0b1001)
        );
        assert_eq!(WorldScopes::from_names(["join", "telepathy"]), None);
        assert!(WorldScopes::GUEST.allows_effect("chat.say"));
        assert!(WorldScopes::GUEST.allows_effect("chat.history"));
        let spectator = WorldScopes::JOIN.union(WorldScopes::INPUT);
        assert!(!spectator.allows_effect("chat.say"));
        assert!(!spectator.allows_effect("chat.history"));
        assert!(spectator.allows_effect("time.now"));
    }

    #[test]
    fn delegated_capabilities_are_narrow_and_die_with_their_parent() {
        let mut core = world(8);
        let root = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let (owner, owner_token) = core
            .issue_capability("Ada", root, CapabilityBounds::default())
            .unwrap();
        let (child, child_token) = core
            .attenuate(
                &owner_token,
                WorldScopes::JOIN.union(WorldScopes::DELEGATE),
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        let (grandchild, grandchild_token) = core
            .attenuate(
                &child_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Cy",
            )
            .unwrap();
        assert!(core.chat(&grandchild_token, "hi").is_err());
        assert!(
            core.attenuate(
                &child_token,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Di"
            )
            .is_err()
        );
        assert!(
            core.attenuate(
                &grandchild_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Ed"
            )
            .is_err()
        );

        let mut revoked = core.revoke_tree(owner.id);
        revoked.sort_unstable();
        assert_eq!(revoked, vec![owner.id, child.id, grandchild.id]);
        assert!(core.snapshot(&child_token).is_err());
        assert!(core.snapshot(&grandchild_token).is_err());
    }

    #[test]
    fn delegation_counts_against_the_participant_limit() {
        let limits = WorldLimits {
            participants: 2,
            ..WorldLimits::default()
        };
        let mut core = WorldCore::new("limit", limits).unwrap();
        let root = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let (_, owner_token) = core
            .issue_capability("Ada", root, CapabilityBounds::default())
            .unwrap();
        core.attenuate(
            &owner_token,
            WorldScopes::JOIN,
            CapabilityBounds::default(),
            "Bo",
        )
        .unwrap();
        assert!(
            core.attenuate(
                &owner_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Cy"
            )
            .is_err()
        );
    }

    #[test]
    fn a_restored_delegation_keeps_its_parent_link() {
        let secret = "cd".repeat(32);
        let token = JoinCapability::from_wire(format!("2.{secret}"));
        let digest = hex_encode(&token_digest(token.expose()));
        let entries = vec![
            JournalEntry::Created {
                world_id: "delegate".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest: "00".repeat(32),
                scopes: WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits(),
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            },
            JournalEntry::Issued {
                issued_at: None,
                participant: Participant {
                    id: 2,
                    display_name: "bo".to_owned(),
                },
                digest,
                scopes: WorldScopes::JOIN.bits(),
                parent: Some(1),
                expires_at: None,
                uses: None,
                used: 0,
                subject: None,
            },
        ];
        let mut restored = WorldCore::restore("delegate", WorldLimits::default(), entries)
            .unwrap()
            .core;
        assert!(restored.authorize(&token, WorldScopes::JOIN).is_ok());
        assert_eq!(restored.revoke_tree(1), vec![1, 2]);
        assert!(restored.authorize(&token, WorldScopes::JOIN).is_err());
    }

    #[test]
    fn foreign_and_malformed_capabilities_are_rejected() {
        let mut left = world(8);
        let right = world(8);
        let (_, left_token) = join(&mut left, "Ada");
        let malformed = JoinCapability("not-a-valid-token".to_owned());
        assert!(left.snapshot(&malformed).is_err());
        assert!(right.snapshot(&left_token).is_err());
    }

    #[test]
    fn names_chat_and_input_are_sanitized_and_bounded() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "\u{1b}[31mAda\n");
        assert_eq!(participant.display_name, "[31mAda");
        assert_eq!(
            core.chat(&token, " hi\u{1b}[31m\n").unwrap().kind,
            WorldEventKind::Chat("hi[31m".to_owned())
        );
        assert!(core.chat(&token, &"x".repeat(2049)).is_err());
        assert!(core.input(&token, &vec![0; 16 * 1024 + 1]).is_err());
        assert!(core.chat(&token, " \n \t ").is_err());
    }

    #[test]
    fn participant_capacity_and_constructor_limits_are_enforced() {
        let mut core = WorldCore::new(
            "small",
            WorldLimits {
                participants: 1,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        join(&mut core, "Ada");
        assert!(
            core.issue_capability("Grace", WorldScopes::GUEST, CapabilityBounds::default())
                .is_err()
        );
        assert!(WorldCore::new("", WorldLimits::default()).is_err());
        assert!(
            WorldCore::new(
                "world",
                WorldLimits {
                    retained_events: 0,
                    ..WorldLimits::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn join_projections_redact_the_install_seed_but_the_journal_keeps_it() {
        // Issue #240: the install seed was serialized to every joiner via
        // `events_after` and the snapshot. The journal (and the admin paths
        // fed from it) must keep the real seed; join projections must not.
        let entries = vec![
            JournalEntry::Created {
                world_id: "test-world".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::Committed {
                event: WorldEvent {
                    sequence: 1,
                    participant_id: 0,
                    kind: WorldEventKind::ProgramInstalled {
                        module_hash: "ab".repeat(32),
                        seed: 424_242,
                        provenance: None,
                        trust: None,
                    },
                },
            },
        ];
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.program.as_ref().map(|program| program.seed),
            Some(424_242),
            "the journal restore path keeps the real seed for the admin"
        );
        let mut core = restored.core;
        assert_eq!(core.active_seed, 424_242, "the core keeps the real seed");
        let (_, token) = join(&mut core, "Ada");
        let page = core.events_after(&token, 0).unwrap();
        assert_eq!(page.events.len(), 1);
        assert!(
            matches!(
                &page.events[0].kind,
                WorldEventKind::ProgramInstalled { module_hash, seed, .. }
                    if module_hash == &"ab".repeat(32) && *seed == 0
            ),
            "events_after must keep the module hash but redact the seed: {:?}",
            page.events[0].kind
        );
        assert!(
            !format!("{page:?}").contains("424242"),
            "events_after leaked the install seed"
        );
        let snapshot = core.snapshot(&token).unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert!(
            !serde_json::to_string(&snapshot).unwrap().contains("424242"),
            "the snapshot leaked the install seed"
        );
    }

    fn scoped(core: &mut WorldCore, name: &str, names: &[&str]) -> JoinCapability {
        let scopes = WorldScopes::from_names(names.iter().copied()).unwrap();
        core.issue_capability(name, scopes, CapabilityBounds::default())
            .unwrap()
            .1
    }

    fn chat_texts(events: &[WorldEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match &event.kind {
                WorldEventKind::Chat(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Chat line, input, chat line, input: sequences 1..=4.
    fn talk(core: &mut WorldCore, speaker: &JoinCapability) {
        core.chat(speaker, "one").unwrap();
        core.input(speaker, b"a").unwrap();
        core.chat(speaker, "two").unwrap();
        core.input(speaker, b"b").unwrap();
    }

    #[test]
    fn chat_visibility_follows_the_viewers_scopes() {
        let mut core = world(16);
        let (_, speaker) = join(&mut core, "Ada");
        let watcher = scoped(&mut core, "watch", &["join"]);
        let reader = scoped(&mut core, "read", &["join", "history"]);
        talk(&mut core, &speaker);

        let sequences = |events: &[WorldEvent]| -> Vec<u64> {
            events.iter().map(|event| event.sequence).collect()
        };
        for (token, sees_chat) in [(&speaker, true), (&reader, true), (&watcher, false)] {
            let page = core.events_after(token, 0).unwrap();
            let snapshot = core.snapshot(token).unwrap();
            for events in [&page.events, &snapshot.events] {
                let (lines, kept): (Vec<&str>, Vec<u64>) = if sees_chat {
                    (vec!["one", "two"], vec![1, 2, 3, 4])
                } else {
                    (Vec::new(), vec![2, 4])
                };
                assert_eq!(chat_texts(events), lines);
                assert_eq!(sequences(events), kept, "non-chat events are kept");
            }
        }
    }

    #[test]
    fn chat_visibility_history_reads_without_sending() {
        let mut core = world(16);
        let (_, speaker) = join(&mut core, "Ada");
        let reader = scoped(&mut core, "read", &["join", "history"]);
        core.chat(&speaker, "hello").unwrap();
        assert!(core.chat(&reader, "hi").is_err(), "history does not send");
        assert!(core.input(&reader, b"a").is_err());
        assert_eq!(
            chat_texts(&core.events_after(&reader, 0).unwrap().events),
            ["hello"]
        );
        assert_eq!(core.current_sequence(), 1, "a refused line commits nothing");
    }

    #[test]
    fn chat_visibility_cursors_advance_across_gaps() {
        let mut core = world(16);
        let (_, speaker) = join(&mut core, "Ada");
        let watcher = scoped(&mut core, "watch", &["join"]);
        core.chat(&speaker, "one").unwrap();
        core.chat(&speaker, "two").unwrap();

        // Only chat so far: nothing to show, yet the cursor moves to the end
        // and retention is judged on the world, not the viewer.
        let page = core.events_after(&watcher, 0).unwrap();
        assert!(page.events.is_empty());
        assert!(!page.needs_snapshot);
        assert_eq!(page.current_sequence, 2);
        assert_eq!(page.oldest_retained_sequence, 1);
        let again = core.events_after(&watcher, page.current_sequence).unwrap();
        assert!(again.events.is_empty() && !again.needs_snapshot);

        core.chat(&speaker, "three").unwrap();
        core.input(&speaker, b"a").unwrap();
        let page = core.events_after(&watcher, page.current_sequence).unwrap();
        assert_eq!(
            page.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            [4]
        );
        // Resuming from the last sequence it was given, across the gap, misses
        // nothing and repeats nothing.
        let last = page.events[0].sequence;
        core.input(&speaker, b"b").unwrap();
        let page = core.events_after(&watcher, last).unwrap();
        assert_eq!(
            page.events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            [5]
        );
    }

    #[test]
    fn chat_visibility_retention_does_not_depend_on_the_viewer() {
        let mut core = world(3);
        let (_, speaker) = join(&mut core, "Ada");
        let watcher = scoped(&mut core, "watch", &["join"]);
        for line in ["one", "two", "three", "four"] {
            core.chat(&speaker, line).unwrap();
        }
        // Sequences 1..=4 committed; 2..=4 retained, all of them chat.
        let page = core.events_after(&watcher, 0).unwrap();
        assert!(page.needs_snapshot, "the cursor fell behind retention");
        assert_eq!(page.oldest_retained_sequence, 2);
        let snapshot = core.snapshot(&watcher).unwrap();
        assert!(snapshot.events.is_empty());
        assert_eq!(snapshot.current_sequence, 4);
        assert_eq!(snapshot.oldest_retained_sequence, 5, "carries no events");
    }

    #[test]
    fn chat_visibility_the_snapshot_budget_counts_only_what_it_carries() {
        let mut core = WorldCore::new(
            "budget",
            WorldLimits {
                snapshot_bytes: 400,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, speaker) = join(&mut core, "Ada");
        let watcher = scoped(&mut core, "watch", &["join"]);
        core.input(&speaker, b"a").unwrap();
        core.input(&speaker, b"b").unwrap();
        for _ in 0..20 {
            core.chat(&speaker, &"x".repeat(100)).unwrap();
        }
        let guest = core.snapshot(&speaker).unwrap();
        assert!(
            guest.events.len() < 22
                && guest
                    .events
                    .iter()
                    .all(|e| matches!(e.kind, WorldEventKind::Chat(_))),
            "chat fills the budget for a viewer who sees it"
        );
        let snapshot = core.snapshot(&watcher).unwrap();
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(|e| e.sequence)
                .collect::<Vec<_>>(),
            [1, 2],
            "withheld chat spends none of the budget"
        );
        assert_eq!(snapshot.oldest_retained_sequence, 1);
    }

    #[test]
    fn chat_visibility_the_install_seed_stays_secret_for_every_scope() {
        let event = WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "ab".repeat(32),
                seed: 424_242,
                provenance: None,
                trust: None,
            },
        };
        for scopes in [WorldScopes::JOIN, WorldScopes::GUEST, WorldScopes::ALL] {
            let shown = project_event_for(scopes, &event).unwrap();
            assert_eq!(
                shown.kind,
                WorldEventKind::ProgramInstalled {
                    module_hash: "ab".repeat(32),
                    seed: 0,
                    provenance: None,
                    trust: None,
                }
            );
        }
        let chat = WorldEvent {
            sequence: 2,
            participant_id: 1,
            kind: WorldEventKind::Chat("hi".to_owned()),
        };
        assert_eq!(project_event_for(WorldScopes::JOIN, &chat), None);
        assert_eq!(
            project_event_for(WorldScopes::CHAT, &chat),
            Some(chat.clone())
        );
        assert_eq!(project_event_for(WorldScopes::HISTORY, &chat), Some(chat));
    }

    #[tokio::test]
    async fn chat_visibility_the_live_stream_is_projected_per_viewer() {
        let core = world(16);
        let world = WorldHandle::spawn(core);
        let (_, guest) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        let (_, watcher) = world.issue("watch", WorldScopes::JOIN).await.unwrap();
        let mut live = world.subscribe();

        world.chat(guest.clone(), "secret").await.unwrap();
        world
            .install_program(
                "cd".repeat(32),
                424_242,
                Box::new(Counter {
                    count: 0,
                    bad: false,
                }),
            )
            .await
            .unwrap();

        let chat = live.recv().await.unwrap();
        let install = live.recv().await.unwrap();
        // The raw stream is the host's: it carries the line and the real seed.
        assert_eq!(chat_texts(std::slice::from_ref(&chat)), ["secret"]);
        assert!(matches!(
            install.kind,
            WorldEventKind::ProgramInstalled { seed: 424_242, .. }
        ));

        // What a participant may be sent is the projection.
        assert_eq!(
            world
                .project_event(watcher.clone(), chat.clone())
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            world
                .project_event(guest.clone(), chat.clone())
                .await
                .unwrap(),
            Some(chat)
        );
        for token in [&guest, &watcher] {
            let shown = world
                .project_event(token.clone(), install.clone())
                .await
                .unwrap()
                .unwrap();
            assert!(
                matches!(shown.kind, WorldEventKind::ProgramInstalled { seed: 0, .. }),
                "{shown:?}"
            );
        }
        // A revoked participant is an error, so its session ends.
        let (revoked, gone) = world.issue("gone", WorldScopes::JOIN).await.unwrap();
        assert!(world.revoke(revoked.id).await.unwrap());
        assert!(world.project_event(gone, install).await.is_err());
    }

    #[test]
    fn chat_visibility_scope_names_attenuation_and_chains() {
        assert_eq!(
            WorldScopes::from_names(["history"]),
            Some(WorldScopes::HISTORY)
        );
        assert_eq!(WorldScopes::HISTORY.bits(), 0b1_0000);
        assert_eq!(WorldScopes::from_names(["join", "telepathy"]), None);
        assert_eq!(WorldScopes::from_bits(0b1_0000), Some(WorldScopes::HISTORY));
        assert_eq!(
            WorldScopes::from_bits(0b10_0000),
            Some(WorldScopes::BUILDER)
        );
        assert_eq!(WorldScopes::from_bits(0b1000_0000), None);

        let join_history = WorldScopes::JOIN.union(WorldScopes::HISTORY);
        let chat_delegator = WorldScopes::GUEST.union(WorldScopes::DELEGATE);
        let history_delegator = join_history.union(WorldScopes::DELEGATE);
        // A reader delegates what it holds and no more; chat includes history.
        assert_eq!(
            history_delegator.attenuate(join_history),
            Some(join_history)
        );
        assert_eq!(
            history_delegator.attenuate(WorldScopes::JOIN),
            Some(WorldScopes::JOIN)
        );
        assert!(history_delegator.attenuate(WorldScopes::GUEST).is_none());
        assert!(
            history_delegator
                .attenuate(WorldScopes::JOIN.union(WorldScopes::CHAT))
                .is_none(),
            "reading does not include sending"
        );
        assert_eq!(chat_delegator.attenuate(join_history), Some(join_history));
        assert!(
            WorldScopes::GUEST.attenuate(join_history).is_none(),
            "no delegate"
        );
        assert!(WorldScopes::GUEST.reads_chat() && join_history.reads_chat());
        assert!(!WorldScopes::JOIN.union(WorldScopes::INPUT).reads_chat());
        assert!(WorldScopes::ALL.contains(WorldScopes::HISTORY));
    }

    #[test]
    fn chat_visibility_a_guest_can_hand_out_a_read_only_link() {
        let mut core = world(16);
        let owner = scoped(&mut core, "owner", &["join", "chat", "input", "delegate"]);
        let (_, reader) = core
            .attenuate(
                &owner,
                WorldScopes::JOIN.union(WorldScopes::HISTORY),
                CapabilityBounds::default(),
                "reader",
            )
            .unwrap();
        let (_, watcher) = core
            .attenuate(
                &owner,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "watch",
            )
            .unwrap();
        core.chat(&owner, "hello").unwrap();
        assert_eq!(
            chat_texts(&core.snapshot(&reader).unwrap().events),
            ["hello"]
        );
        assert!(chat_texts(&core.snapshot(&watcher).unwrap().events).is_empty());
        assert!(core.chat(&reader, "hi").is_err());
    }

    #[test]
    fn snapshots_do_not_include_capability_secret() {
        let mut core = world(8);
        let (participant, token) = join(&mut core, "Ada");
        let serialized = format!("{:?}", core.snapshot(&token).unwrap());
        assert!(!serialized.contains(token.expose()));
        assert!(serialized.contains(&participant.display_name));
        assert!(!format!("{token:?}").contains(token.expose()));
    }

    #[tokio::test]
    async fn actor_serializes_concurrent_event_mutations() {
        let actor = WorldHandle::spawn(world(32));
        let (_, token) = actor.issue("Ada", WorldScopes::GUEST).await.unwrap();
        let first = actor.chat(token.clone(), "one");
        let second = actor.input(token.clone(), b"two".to_vec());
        let third = actor.chat(token.clone(), "three");
        let (one, two, three) = tokio::join!(first, second, third);
        let mut sequences = [
            one.unwrap().sequence,
            two.unwrap().sequence,
            three.unwrap().sequence,
        ];
        sequences.sort_unstable();
        assert_eq!(sequences, [1, 2, 3]);
        assert_eq!(actor.snapshot(token).await.unwrap().current_sequence, 3);
    }

    /// Counts inputs; records are the count, or `bad` after an input "bad".
    struct Counter {
        count: u64,
        bad: bool,
    }

    impl WorldProgram for Counter {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(bytes) = &event.kind {
                self.count += 1;
                self.bad = bytes.as_slice() == b"bad";
            }
            Ok(())
        }
        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(if self.bad {
                "[1,2,3]".to_owned()
            } else {
                format!("{{\"count\":{}}}", self.count)
            }))
        }
    }

    #[tokio::test]
    async fn records_read_returns_a_bounded_prefix_scoped_page() {
        struct ReaderProgram {
            asks: u64,
            read_key: String,
        }
        impl WorldProgram for ReaderProgram {
            fn update(&mut self, event: &WorldEvent) -> Result<()> {
                match &event.kind {
                    WorldEventKind::Input(bytes) => {
                        if bytes.as_slice() == b"read" {
                            self.asks += 1;
                        }
                    }
                    WorldEventKind::EntityInput { input, .. } => {
                        if input.as_slice() == b"read" {
                            self.asks += 1;
                        }
                    }
                    WorldEventKind::System { event } => {
                        let value: serde_json::Value =
                            serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                        if value["cap"] == "result"
                            && let Some(key) = value["value"]["records"][0]["key"].as_str()
                        {
                            self.read_key = key.to_owned();
                        }
                    }
                    WorldEventKind::Chat(_)
                    | WorldEventKind::ProgramInstalled { .. }
                    | WorldEventKind::Notification { .. } => {}
                }
                Ok(())
            }

            fn records(&mut self) -> Result<Option<String>> {
                Ok(Some(
                    serde_json::json!({
                        "count": self.asks,
                        "read": self.read_key,
                    })
                    .to_string(),
                ))
            }

            fn wants(&mut self) -> Result<Option<String>> {
                if self.asks == 0 {
                    return Ok(Some("{}".to_owned()));
                }
                Ok(Some(
                    serde_json::json!({
                        "requests": [{
                            "id": format!("read-{}", self.asks),
                            "cap": "records.read",
                            "args": {"prefix": "co", "after": ""},
                        }],
                    })
                    .to_string(),
                ))
            }
        }

        let world = WorldHandle::spawn_with_program(
            world(16),
            Box::new(ReaderProgram {
                asks: 0,
                read_key: String::new(),
            }),
        );
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();

        // Ungranted: the request is refused and counted.
        world.input(ann.clone(), b"read".to_vec()).await.unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let report = world.caps_report().await.unwrap();
            if report
                .usage
                .get("records.read")
                .is_some_and(|use_| use_.denied > 0)
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no refusal recorded");
            tokio::task::yield_now().await;
        }

        // Granted: the program reads its own published `count` record.
        world
            .update_caps(vec!["records.read".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(ann, b"read".to_vec()).await.unwrap();
        loop {
            let record = world.records().1["read"].as_str().unwrap_or("").to_owned();
            if record == "count" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the program never read its record: {record:?}"
            );
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn records_follow_committed_inputs_and_bad_output_fails_closed() {
        let world = WorldHandle::spawn_with_program(
            world(16),
            Box::new(Counter {
                count: 0,
                bad: false,
            }),
        );
        assert_eq!(world.records().1["count"], 0);
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        let event = world.input(ann.clone(), b"go".to_vec()).await.unwrap();
        let (sequence, records) = world.records();
        assert_eq!(
            (sequence, records["count"].as_u64()),
            (event.sequence, Some(1))
        );

        // Records that are not a JSON object reject the input and poison the
        // program; the previous records stay published.
        assert!(world.input(ann.clone(), b"bad".to_vec()).await.is_err());
        assert_eq!(world.records().1["count"], 1);
        assert!(world.input(ann.clone(), b"go".to_vec()).await.is_err());
        let snapshot = world.snapshot(ann.clone()).await.unwrap();
        assert_eq!(
            snapshot.current_sequence, event.sequence,
            "nothing committed"
        );

        // A replacement with valid records restores service.
        world
            .install_program(
                "00".repeat(32),
                0,
                Box::new(Counter {
                    count: 10,
                    bad: false,
                }),
            )
            .await
            .unwrap();
        assert_eq!(world.records().1["count"], 10);
        world.input(ann, b"go".to_vec()).await.unwrap();
        assert_eq!(world.records().1["count"], 11);
    }

    #[tokio::test]
    async fn installs_with_invalid_records_are_refused() {
        let world = WorldHandle::spawn(world(16));
        let err = world
            .install_program(
                "00".repeat(32),
                0,
                Box::new(Counter {
                    count: 0,
                    bad: true,
                }),
            )
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("records"), "{err:#}");
        let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        assert_eq!(world.snapshot(ann).await.unwrap().active_module_hash, None);
    }

    #[test]
    fn record_keys_and_sizes_are_bounded() {
        assert!(parse_records(r#"{"a":1,"b":{"c":[true]}}"#).is_ok());
        assert!(parse_records("[]").is_err());
        assert!(parse_records(r#"{"":1}"#).is_err());
        assert!(parse_records("{\"a\\u0000b\":1}").is_err());
        let long_key = format!(r#"{{"{}":1}}"#, "k".repeat(MAX_RECORD_KEY_BYTES + 1));
        assert!(parse_records(&long_key).is_err());
        let big_value = format!(r#"{{"k":"{}"}}"#, "v".repeat(MAX_RECORD_VALUE_BYTES));
        assert!(parse_records(&big_value).is_err());
        let many: Records = (0..=MAX_RECORDS)
            .map(|i| (i.to_string(), 0.into()))
            .collect();
        assert!(parse_records(&serde_json::to_string(&many).unwrap()).is_err());
    }

    #[test]
    fn records_refuse_duplicate_keys() {
        // Oracle: the written document. Any repeated object key, at any
        // depth, is a parse error naming the repeated key (#6473, #6520).
        for text in [
            r#"{"a":1,"a":2}"#,
            r#"{"a":{"b":1,"b":2}}"#,
            r#"{"a":[{"b":1,"b":2}]}"#,
        ] {
            let err = parse_records(text).unwrap_err();
            assert!(
                format!("{err:#}").contains("duplicate JSON object key"),
                "{err:#}"
            );
        }
        // An oversized value hidden behind a small duplicate never reaches
        // the size check as a collapsed survivor: the duplicate refuses it.
        let hidden = format!(r#"{{"k":"{}","k":1}}"#, "v".repeat(MAX_RECORD_VALUE_BYTES));
        assert!(parse_records(&hidden).is_err());
        // 4097 raw entries collapsing to 4096 distinct keys are refused as
        // duplicates, so the count cap can no longer be evaded by collapse.
        let mut raw = String::from("{");
        for i in 0..=MAX_RECORDS {
            if i > 0 {
                raw.push(',');
            }
            raw.push_str(&format!("\"{i}\":0"));
        }
        raw.push_str(",\"0\":0}");
        assert!(parse_records(&raw).is_err());
        // Distinct-key inputs parse identically to before.
        let ok = parse_records(r#"{"a":1,"b":{"c":[true]}}"#).unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(ok["b"]["c"][0], serde_json::Value::Bool(true));
    }

    #[test]
    fn snapshots_carry_the_newest_events_that_fit_their_budget() {
        let mut core = WorldCore::new(
            "budget",
            WorldLimits {
                snapshot_bytes: 1024,
                input_bytes: 64,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let (_, token) = join(&mut core, "ann");
        for _ in 0..20u8 {
            core.input(&token, &[b'x'; 48]).unwrap();
        }
        let snapshot = core.snapshot(&token).unwrap();
        assert_eq!(snapshot.current_sequence, 20);
        assert!(!snapshot.events.is_empty());
        assert!(
            serde_json::to_vec(&snapshot.events).unwrap().len() <= 1024,
            "events fit the budget"
        );
        assert!(
            snapshot.events.len() < 20,
            "an oversized log is truncated to the newest events"
        );
        let first = snapshot.events.first().unwrap().sequence;
        assert_eq!(snapshot.oldest_retained_sequence, first);
        assert_eq!(snapshot.events.last().unwrap().sequence, 20);
        // The snapshot is a valid rebase point: continuing from its cursor
        // yields only new events.
        assert!(
            core.events_after(&token, snapshot.current_sequence)
                .unwrap()
                .events
                .is_empty()
        );
    }

    /// A program whose `view` traps exactly once, then recovers. Used to prove
    /// a trapping view does not poison the world (F11.17).
    #[derive(Default)]
    struct TrapsOnce {
        calls: u64,
    }

    impl WorldProgram for TrapsOnce {
        fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
            self.calls += 1;
            if self.calls == 1 {
                anyhow::bail!("boom on the first view");
            }
            Ok(Some("recovered view".to_owned()))
        }
    }

    #[tokio::test]
    async fn a_trapping_view_degrades_to_an_error_and_recovers_on_the_next_event() {
        let mut core = WorldCore::new("trap", WorldLimits::default()).unwrap();
        let (_, token) = join(&mut core, "ann");
        let mut actor = WorldActor::for_test(core, Box::new(TrapsOnce::default()), None);

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::View {
                token: token.clone(),
                reply,
            })
            .await;
        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Err(e) if e.to_string().contains("view failed")),
            "the trap is reported as a view failure: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "a trapping view must not poison the world"
        );
        assert_eq!(actor.view_failures, 1, "the trap is counted");

        // The next event retries the view and recovers, no reinstall needed.
        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::View {
                token: token.clone(),
                reply,
            })
            .await;
        let second = rx.await.unwrap();
        assert!(
            matches!(&second, Ok(Some(text)) if text == "recovered view"),
            "the next view recovers: {second:?}"
        );
        assert_eq!(actor.view_failures, 1, "only the one trap is counted");
    }

    struct OversizedViewOnce {
        calls: u64,
        oversized_bytes: usize,
    }

    impl WorldProgram for OversizedViewOnce {
        fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
            self.calls += 1;
            if self.calls == 1 {
                Ok(Some("x".repeat(self.oversized_bytes)))
            } else {
                Ok(Some("recovered view".to_owned()))
            }
        }
    }

    #[tokio::test]
    async fn an_oversized_view_degrades_to_an_error_and_recovers() {
        let limits = WorldLimits::default();
        let presentation_bytes = limits.presentation_bytes;
        let mut core = WorldCore::new("oversized-view", limits).unwrap();
        let (_, token) = join(&mut core, "ann");
        let mut actor = WorldActor::for_test(
            core,
            Box::new(OversizedViewOnce {
                calls: 0,
                oversized_bytes: presentation_bytes + 1,
            }),
            None,
        );

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::View {
                token: token.clone(),
                reply,
            })
            .await;
        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Err(error) if error.to_string().contains(&format!("world presentation exceeds {presentation_bytes} bytes"))),
            "the oversized view names the presentation limit: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "an oversized view must not poison the world"
        );
        assert_eq!(actor.view_failures, 1, "the oversized view is counted");

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor.handle(WorldCommand::View { token, reply }).await;
        let second = rx.await.unwrap();
        assert!(
            matches!(&second, Ok(Some(text)) if text == "recovered view"),
            "the next view recovers: {second:?}"
        );
        assert_eq!(actor.view_failures, 1, "only the oversized view is counted");
    }

    #[tokio::test]
    async fn an_oversized_host_view_degrades_to_an_error_and_recovers() {
        let limits = WorldLimits::default();
        let presentation_bytes = limits.presentation_bytes;
        let core = WorldCore::new("oversized-host-view", limits).unwrap();
        let mut actor = WorldActor::for_test(
            core,
            Box::new(OversizedViewOnce {
                calls: 0,
                oversized_bytes: presentation_bytes + 1,
            }),
            None,
        );

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor.handle(WorldCommand::HostView { reply }).await;
        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Err(error) if error.to_string().contains(&format!("world presentation exceeds {presentation_bytes} bytes"))),
            "the oversized host view names the presentation limit: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "an oversized host view must not poison the world"
        );
        assert_eq!(actor.view_failures, 1, "the oversized host view is counted");

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor.handle(WorldCommand::HostView { reply }).await;
        let second = rx.await.unwrap();
        assert!(
            matches!(&second, Ok((_, Some(text))) if text == "recovered view"),
            "the next host view recovers: {second:?}"
        );
        assert_eq!(
            actor.view_failures, 1,
            "only the oversized host view is counted"
        );
    }

    #[derive(Default)]
    struct OversizedLayout;

    impl WorldProgram for OversizedLayout {
        fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
            Ok(Some(serde_json::json!({"kind": "screen", "version": 1, "width": 10, "height": 10, "cells": []}).to_string()))
        }

        fn layout(
            &mut self,
            _viewer: &Participant,
            width: u16,
            _height: u16,
        ) -> Result<Option<String>> {
            if width > 100 {
                // Return an artificially huge response exceeding 8192 bytes
                let pad = "x".repeat(9000);
                Ok(Some(serde_json::json!({"kind": "screen", "version": 1, "width": 120, "height": 50, "cells": [], "pad": pad}).to_string()))
            } else {
                Ok(Some(serde_json::json!({"kind": "screen", "version": 1, "width": width, "height": 10, "cells": []}).to_string()))
            }
        }
    }

    #[tokio::test]
    async fn an_oversized_layout_degrades_to_a_view_fallback_and_recovers() {
        let mut core = WorldCore::new("oversized", WorldLimits::default()).unwrap();
        let (_, token) = join(&mut core, "ann");
        let mut actor = WorldActor::for_test(core, Box::new(OversizedLayout::default()), None);

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ViewAt {
                token: token.clone(),
                width: 120,
                height: 50,
                reply,
            })
            .await;

        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Ok(Some(text)) if text.contains("\"width\":10")),
            "the oversized layout falls back to the cropped default view: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "an oversized layout must not poison the world"
        );
        assert_eq!(
            actor.view_failures, 1,
            "the fallback triggers a view failure count"
        );
    }

    struct OversizedFallbackText;

    impl WorldProgram for OversizedFallbackText {
        fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
            // A non-screen fallback text far beyond the presentation limit.
            Ok(Some("y".repeat(9000)))
        }

        fn layout(
            &mut self,
            _viewer: &Participant,
            _width: u16,
            _height: u16,
        ) -> Result<Option<String>> {
            let pad = "x".repeat(9000);
            Ok(Some(serde_json::json!({"kind": "screen", "version": 1, "width": 120, "height": 50, "cells": [], "pad": pad}).to_string()))
        }
    }

    #[tokio::test]
    async fn an_over_limit_fallback_text_is_bounded_by_presentation_bytes() {
        let mut core = WorldCore::new("oversized-fallback-text", WorldLimits::default()).unwrap();
        let (_, token) = join(&mut core, "ann");
        let mut actor = WorldActor::for_test(core, Box::new(OversizedFallbackText), None);

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ViewAt {
                token,
                width: 120,
                height: 50,
                reply,
            })
            .await;

        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Err(error) if error.to_string().contains("world presentation exceeds 8192 bytes")),
            "an over-limit fallback text is refused, not delivered: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "an over-limit fallback must not poison the world"
        );
        assert_eq!(
            actor.view_failures, 2,
            "the oversized layout and the over-limit fallback are both counted"
        );
    }

    struct OversizedFallbackScreen;

    impl WorldProgram for OversizedFallbackScreen {
        fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
            // A full 120x50 screen whose cropped serialization is far beyond
            // the presentation limit (6000 populated cells).
            let cell =
                serde_json::json!({"symbol": "x", "fg": 1, "bg": 2, "bold": true, "columns": 1});
            let cells: Vec<_> = std::iter::repeat_n(cell, 120 * 50).collect();
            Ok(Some(
                serde_json::json!({"kind": "screen", "version": 1, "width": 120, "height": 50, "cells": cells})
                    .to_string(),
            ))
        }

        fn layout(
            &mut self,
            _viewer: &Participant,
            _width: u16,
            _height: u16,
        ) -> Result<Option<String>> {
            let pad = "x".repeat(9000);
            Ok(Some(serde_json::json!({"kind": "screen", "version": 1, "width": 120, "height": 50, "cells": [], "pad": pad}).to_string()))
        }
    }

    #[tokio::test]
    async fn an_over_limit_cropped_fallback_screen_is_bounded_by_presentation_bytes() {
        let mut core = WorldCore::new("oversized-fallback-screen", WorldLimits::default()).unwrap();
        let (_, token) = join(&mut core, "ann");
        let mut actor = WorldActor::for_test(core, Box::new(OversizedFallbackScreen), None);

        let (reply, rx) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::ViewAt {
                token,
                width: 120,
                height: 50,
                reply,
            })
            .await;

        let first = rx.await.unwrap();
        assert!(
            matches!(&first, Err(error) if error.to_string().contains("world presentation exceeds 8192 bytes")),
            "an over-limit cropped fallback screen is refused, not delivered: {first:?}"
        );
        assert!(
            actor.program_healthy,
            "an over-limit cropped fallback must not poison the world"
        );
        assert_eq!(
            actor.view_failures, 2,
            "the oversized layout and the over-limit cropped fallback are both counted"
        );
    }

    #[test]
    fn signed_moves_chain_verify_and_replay_the_journal_order() {
        use p2panda_core::{Hash, SigningKey};

        let mut core = WorldCore::new("ttt", WorldLimits::default()).unwrap();
        let (_, token_x) = core
            .issue_capability("x", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let (_, token_o) = core
            .issue_capability("o", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let key_x = SigningKey::from_bytes(Hash::digest(b"x key").as_bytes());
        let key_o = SigningKey::from_bytes(Hash::digest(b"o key").as_bytes());
        let author_x = *key_x.verifying_key().as_bytes();
        let author_o = *key_o.verifying_key().as_bytes();
        core.bind_author_key(&token_x, author_x).unwrap();
        core.bind_author_key(&token_o, author_o).unwrap();

        // Once a key is bound, the unsigned path refuses the participant.
        assert!(core.input(&token_x, b"cell-0").is_err());

        // A tic-tac-toe game through the world path: alternating signed
        // moves, each chained to the player's previous move.
        let script: [(&JoinCapability, &SigningKey, &[u8]); 5] = [
            (&token_x, &key_x, b"cell-0"),
            (&token_o, &key_o, b"cell-4"),
            (&token_x, &key_x, b"cell-1"),
            (&token_o, &key_o, b"cell-3"),
            (&token_x, &key_x, b"cell-2"),
        ];
        let mut seq: std::collections::HashMap<u64, u32> = Default::default();
        let mut frontier: std::collections::HashMap<u64, Hash> = Default::default();
        for (token, key, input) in script {
            let author = *key.verifying_key().as_bytes();
            let participant = if author == author_x { 1 } else { 2 };
            let n = seq.entry(participant).or_default();
            let signed =
                signed::sign_move(key, "ttt", *n, frontier.get(&participant).copied(), input)
                    .unwrap();
            let event = core.input_signed(token, input, &signed.bytes).unwrap();
            assert_eq!(event.kind, WorldEventKind::Input(input.to_vec()));
            frontier.insert(participant, signed.id);
            *n += 1;
        }

        // The accepted records cross-reference world sequence and chain
        // position, and replaying each author's chain against the
        // journal's input bytes reproduces the journal order.
        let records = core.signed_moves().to_vec();
        assert_eq!(records.len(), 5);
        let mut chains: std::collections::HashMap<u64, signed::MoveChain> = Default::default();
        for record in &records {
            let journal_event = core
                .events
                .iter()
                .find(|event| event.sequence == record.world_sequence)
                .unwrap();
            let WorldEventKind::Input(input) = &journal_event.kind else {
                panic!("signed move record does not name an input event");
            };
            assert_eq!(journal_event.participant_id, record.participant_id);
            chains
                .entry(record.participant_id)
                .or_default()
                .verify_and_accept(&record.bytes, "ttt", input, &record.author)
                .unwrap();
        }

        // Tampering: a move signed over one input does not verify for
        // another, and a mutated signed byte string does not verify at
        // all. Neither advances the author's chain.
        let next =
            signed::sign_move(&key_x, "ttt", 3, frontier.get(&1).copied(), b"cell-8").unwrap();
        assert!(core.input_signed(&token_x, b"cell-7", &next.bytes).is_err());
        let mut mutated = next.bytes.clone();
        mutated[12] ^= 0xff;
        assert!(core.input_signed(&token_x, b"cell-8", &mutated).is_err());
        core.input_signed(&token_x, b"cell-8", &next.bytes).unwrap();

        // Equivocation: a rival move signed at the same author sequence
        // is a valid chain event on its own (the fork-proof material the
        // event store's ingest rules consume), but the world refuses it
        // once the first move at that sequence has been accepted.
        let rival =
            signed::sign_move(&key_x, "ttt", 3, frontier.get(&1).copied(), b"cell-6").unwrap();
        assert!(crate::chain::decode(&rival.bytes).is_ok());
        assert!(
            core.input_signed(&token_x, b"cell-6", &rival.bytes)
                .is_err()
        );
    }

    /// Play a three-move signed game on a fresh core: x moves twice,
    /// o once. Returns the core, both tokens, both keys, and the move
    /// frontier per participant, for the #4346 journal tests.
    fn signed_game() -> (
        WorldCore,
        JoinCapability,
        JoinCapability,
        p2panda_core::SigningKey,
        p2panda_core::SigningKey,
        std::collections::HashMap<u64, p2panda_core::Hash>,
    ) {
        use p2panda_core::{Hash, SigningKey};

        let mut core = WorldCore::new("ttt-journal", WorldLimits::default()).unwrap();
        let (px, token_x) = core
            .issue_capability("x", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let (po, token_o) = core
            .issue_capability("o", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let key_x = SigningKey::from_bytes(Hash::digest(b"x key").as_bytes());
        let key_o = SigningKey::from_bytes(Hash::digest(b"o key").as_bytes());
        core.bind_author_key(&token_x, *key_x.verifying_key().as_bytes())
            .unwrap();
        core.bind_author_key(&token_o, *key_o.verifying_key().as_bytes())
            .unwrap();
        let script: [(&JoinCapability, &SigningKey, u64, u32, &[u8]); 3] = [
            (&token_x, &key_x, px.id, 0, b"cell-0"),
            (&token_o, &key_o, po.id, 0, b"cell-4"),
            (&token_x, &key_x, px.id, 1, b"cell-1"),
        ];
        let mut frontier: std::collections::HashMap<u64, Hash> = Default::default();
        for (token, key, participant, seq, input) in script {
            let signed = signed::sign_move(
                key,
                "ttt-journal",
                seq,
                frontier.get(&participant).copied(),
                input,
            )
            .unwrap();
            core.input_signed(token, input, &signed.bytes).unwrap();
            frontier.insert(participant, signed.id);
        }
        (core, token_x, token_o, key_x, key_o, frontier)
    }

    /// The journal the actor persists for `core`'s signed game (#4346):
    /// header, issued entries, bindings, then each committed input
    /// followed by its accepted-move entry.
    fn signed_journal(core: &WorldCore) -> Vec<JournalEntry> {
        let mut entries = vec![JournalEntry::Created {
            world_id: "ttt-journal".to_owned(),
            version: MAX_JOURNAL_VERSION,
        }];
        let mut ids: Vec<u64> = core.author_keys.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            entries.extend(core.issued_entry(id));
            entries.push(JournalEntry::AuthorKeyBound {
                participant_id: id,
                author: hex_encode(&core.author_keys[&id]),
            });
        }
        for event in &core.events {
            entries.push(JournalEntry::Committed {
                event: event.clone(),
            });
            for record in core
                .signed_moves()
                .iter()
                .filter(|record| record.world_sequence == event.sequence)
            {
                entries.push(JournalEntry::SignedMoveAccepted {
                    record: JournalSignedMove::from_record(record),
                });
            }
        }
        entries
    }

    #[test]
    fn signed_play_survives_journal_restore_and_compaction() {
        let (core, token_x, _token_o, key_x, _key_o, frontier) = signed_game();
        let x_id = core
            .author_keys
            .iter()
            .find(|(_, key)| **key == *key_x.verifying_key().as_bytes())
            .map(|(id, _)| *id)
            .unwrap();
        let records = core.signed_moves().to_vec();
        assert_eq!(records.len(), 3);

        // Restart from the journal: bindings, frontiers, and records
        // are rebuilt, and replay re-verified every move.
        let restored =
            WorldCore::restore("ttt-journal", WorldLimits::default(), signed_journal(&core))
                .unwrap();
        let mut restored = restored.core;
        assert_eq!(restored.signed_moves(), records.as_slice());
        assert_eq!(restored.author_keys, core.author_keys);
        assert_eq!(restored.move_chains[&x_id].next_seq(), 2);
        assert_eq!(
            restored.move_chains[&x_id].frontier(),
            frontier.get(&x_id).copied()
        );

        // The chain continues at the restored position without
        // re-anchoring: x's next move is sequence 2 on the frontier.
        let next = signed::sign_move(
            &key_x,
            "ttt-journal",
            2,
            frontier.get(&x_id).copied(),
            b"cell-2",
        )
        .unwrap();
        restored
            .input_signed(&token_x, b"cell-2", &next.bytes)
            .unwrap();

        // And a rival at an already-accepted sequence is still refused
        // after the restart.
        let rival = signed::sign_move(
            &key_x,
            "ttt-journal",
            2,
            frontier.get(&x_id).copied(),
            b"cell-5",
        )
        .unwrap();
        assert!(
            restored
                .input_signed(&token_x, b"cell-5", &rival.bytes)
                .is_err()
        );

        // Compaction keeps the whole signed state: checkpoint the
        // restored core (its four accepted moves now include the
        // post-restart one) and restore once more.
        restored.active_module_hash = Some("module-hash".to_owned());
        let compacted = restored.checkpoint_entries("state".to_owned()).unwrap();
        let restored2 = WorldCore::restore("ttt-journal", WorldLimits::default(), compacted)
            .unwrap()
            .core;
        assert_eq!(restored2.signed_moves(), restored.signed_moves());
        assert_eq!(restored2.author_keys, restored.author_keys);
        assert_eq!(restored2.move_chains[&x_id].next_seq(), 3);
        assert_eq!(restored2.move_chains[&x_id].frontier(), Some(next.id));
    }

    #[test]
    fn journal_restore_refuses_a_tampered_signed_move() {
        let (core, ..) = signed_game();
        let mut entries = signed_journal(&core);
        // Flip a byte inside the last accepted move's signed bytes: the
        // record no longer matches its bytes, so replay must refuse
        // the whole journal rather than restore a forged chain.
        let last = entries
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                JournalEntry::SignedMoveAccepted { record } => Some(record),
                _ => None,
            })
            .unwrap();
        let mut bytes = last.bytes.clone();
        bytes.replace_range(0..2, "ff");
        last.bytes = bytes;
        assert!(WorldCore::restore("ttt-journal", WorldLimits::default(), entries).is_err());
    }

    #[test]
    fn journal_restore_refuses_a_signed_move_referencing_a_future_sequence() {
        let (core, ..) = signed_game();
        let mut entries = signed_journal(&core);
        // Point the last accepted move at a world sequence that has not
        // committed at that point in the replay: header-only verification
        // must not treat a future reference as aged-out history.
        let last = entries
            .iter_mut()
            .rev()
            .find_map(|entry| match entry {
                JournalEntry::SignedMoveAccepted { record } => Some(record),
                _ => None,
            })
            .unwrap();
        last.world_sequence = 999;
        assert!(WorldCore::restore("ttt-journal", WorldLimits::default(), entries).is_err());
    }

    #[test]
    fn journal_restore_refuses_a_signed_move_referencing_another_participants_input() {
        let (core, ..) = signed_game();
        let mut entries = signed_journal(&core);
        // The first accepted move is x's at world sequence 1; sequence 2
        // is o's committed input. Pointing x's record at it is a false
        // cross-reference, not an input that aged out of the window.
        let first = entries
            .iter_mut()
            .find_map(|entry| match entry {
                JournalEntry::SignedMoveAccepted { record } => Some(record),
                _ => None,
            })
            .unwrap();
        assert_eq!(first.world_sequence, 1);
        first.world_sequence = 2;
        assert!(WorldCore::restore("ttt-journal", WorldLimits::default(), entries).is_err());
    }

    #[test]
    fn signed_play_entries_require_journal_version_9() {
        // Issue #4346: signed-play kinds are the first entries a
        // version-8 binary cannot read, so they pin the header to
        // version 9 under the #227 bump-on-first-write contract
        // (renumbered from 8 when PR #9037 was rebased over the
        // #1355 recurring-schedule bump, which took version 8).
        for entry in [
            JournalEntry::AuthorKeyBound {
                participant_id: 1,
                author: "00".repeat(32),
            },
            JournalEntry::SignedMoveAccepted {
                record: JournalSignedMove {
                    world_sequence: 1,
                    participant_id: 1,
                    author: "00".repeat(32),
                    author_seq: 0,
                    event_id: "00".repeat(32),
                    bytes: String::new(),
                },
            },
        ] {
            assert_eq!(
                journal_version_required_by(&entry),
                JOURNAL_VERSION_SIGNED_PLAY,
                "{entry:?}"
            );
        }
    }

    #[test]
    fn ingested_signed_moves_yield_a_fork_proof_for_a_rival() {
        use id_events::event_store::{EventStore, Ingest};
        use p2panda_core::{Hash, SigningKey};

        let mut core = WorldCore::new("ttt-ingest", WorldLimits::default()).unwrap();
        let (_, token) = core
            .issue_capability("x", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let key = SigningKey::from_bytes(Hash::digest(b"ingest x").as_bytes());
        let author = *key.verifying_key().as_bytes();
        core.bind_author_key(&token, author).unwrap();
        let first = signed::sign_move(&key, "ttt-ingest", 0, None, b"cell-0").unwrap();
        core.input_signed(&token, b"cell-0", &first.bytes).unwrap();
        let second = signed::sign_move(&key, "ttt-ingest", 1, Some(first.id), b"cell-1").unwrap();
        core.input_signed(&token, b"cell-1", &second.bytes).unwrap();

        // The hand-off: every accepted move goes into the event store.
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            EventStore::open(dir.path(), id_core::waiting::WaitingLimits::default(), 0).unwrap();
        let outcomes = core.ingest_signed_moves(&mut store, 0).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome, Ingest::Accepted { .. })),
            "{outcomes:?}"
        );
        assert!(store.fork_proof(&author).unwrap().is_none());

        // A rival signed move at an already-accepted sequence — valid
        // chain material the world itself refused — produces the
        // store's fork proof once ingested.
        let rival = signed::sign_move(&key, "ttt-ingest", 1, Some(first.id), b"cell-6").unwrap();
        let outcome = store.ingest(&rival.bytes, 0).unwrap();
        assert!(
            matches!(outcome, Ingest::Fork { seq: 1, .. }),
            "{outcome:?}"
        );
        let proof = store.fork_proof(&author).unwrap().unwrap();
        assert_eq!(proof.seq, 1);
    }

    #[tokio::test]
    async fn signed_input_flows_through_the_actor() {
        use p2panda_core::{Hash, SigningKey};

        let core = WorldCore::new("ttt-actor", WorldLimits::default()).unwrap();
        let handle = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = handle.issue("x", WorldScopes::GUEST).await.unwrap();
        let key = SigningKey::from_bytes(Hash::digest(b"actor x").as_bytes());
        handle
            .bind_author_key(token.clone(), *key.verifying_key().as_bytes())
            .await
            .unwrap();
        assert!(
            handle
                .input(token.clone(), b"cell-0".to_vec())
                .await
                .is_err()
        );
        let signed = signed::sign_move(&key, "ttt-actor", 0, None, b"cell-0").unwrap();
        let event = handle
            .input_signed(token, b"cell-0".to_vec(), signed.bytes)
            .await
            .unwrap();
        assert_eq!(event.kind, WorldEventKind::Input(b"cell-0".to_vec()));
    }
    // ---- Phase 3 (#3805, design #2819): handoff at the actor boundary ----

    struct LeaseWorld {
        handle: WorldHandle,
        token: JoinCapability,
        journal: TransferJournal,
        clock: Arc<ManualClock>,
        program_updates: Arc<AtomicU64>,
    }

    struct LeaseTestProgram(Arc<AtomicU64>);

    impl WorldProgram for LeaseTestProgram {
        fn update(&mut self, _event: &WorldEvent) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(None)
        }
    }

    async fn lease_world() -> LeaseWorld {
        let clock = Arc::new(ManualClock::new(1_000));
        let core = WorldCore::new_with_clock("w", WorldLimits::default(), clock.clone()).unwrap();
        let journal = TransferJournal(Arc::new(std::sync::Mutex::new(vec![lease_header()])));
        let program_updates = Arc::new(AtomicU64::new(0));
        let handle = WorldHandle::spawn_durable(
            core,
            Box::new(LeaseTestProgram(program_updates.clone())),
            Some(Box::new(journal.clone())),
        );
        let (_, token) = handle.issue("ann", WorldScopes::GUEST).await.unwrap();
        LeaseWorld {
            handle,
            token,
            journal,
            clock,
            program_updates,
        }
    }

    fn committed_inputs(journal: &TransferJournal) -> usize {
        journal
            .entries()
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    JournalEntry::Committed { event }
                        if matches!(event.kind, WorldEventKind::Input(_))
                )
            })
            .count()
    }

    fn lease_error(error: &anyhow::Error) -> Option<LeaseRejectionReason> {
        error.downcast_ref::<LeaseRejectionReason>().copied()
    }

    fn lease_entries(journal: &TransferJournal) -> Vec<JournalEntry> {
        journal
            .entries()
            .into_iter()
            .filter(|entry| matches!(entry, JournalEntry::LeaseHandedOff { .. }))
            .collect()
    }

    #[tokio::test]
    async fn do_to_native_handoff_fences_the_do_executor_and_refuses_queued_commands() {
        let w = lease_world().await;
        w.handle
            .input(w.token.clone(), b"0".to_vec())
            .await
            .unwrap();
        assert_eq!(committed_inputs(&w.journal), 1);
        assert_eq!(w.program_updates.load(Ordering::SeqCst), 1);

        // The handoff is queued ahead of two in-flight commands: they were
        // received by the DO executor but not journaled before the boundary.
        let (handoff, first, second) = tokio::join!(
            w.handle.handoff_write_lease("native-1", 1, Some(31_000)),
            w.handle.input(w.token.clone(), b"0".to_vec()),
            w.handle.chat(w.token.clone(), "late"),
        );
        assert_eq!(handoff.unwrap(), 1);
        assert_eq!(
            lease_error(&first.unwrap_err()),
            Some(LeaseRejectionReason::NotHolder)
        );
        assert_eq!(
            lease_error(&second.unwrap_err()),
            Some(LeaseRejectionReason::NotHolder)
        );
        assert_eq!(
            w.program_updates.load(Ordering::SeqCst),
            1,
            "a command behind the handoff must not execute in the old program"
        );

        // The old writer's durable prefix ends at the control record; no
        // committed command follows that boundary from the DO actor.
        assert_eq!(committed_inputs(&w.journal), 1);
        let entries = w.journal.entries();
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::LeaseHandedOff {
                from,
                to,
                epoch: 1,
                position: 1,
                ..
            }) if from == DO_WRITE_LEASE_HOLDER && to == "native-1"
        ));
        assert!(
            !entries
                .iter()
                .skip_while(|entry| !matches!(entry, JournalEntry::LeaseHandedOff { .. }))
                .any(|entry| matches!(entry, JournalEntry::Committed { .. }))
        );

        // The DO cannot hand off again: it no longer holds the lease.
        assert!(
            w.handle
                .handoff_write_lease("native-2", 2, Some(31_000))
                .await
                .is_err()
        );
        assert_eq!(lease_entries(&w.journal).len(), 1);
    }

    #[tokio::test]
    async fn native_to_do_handback_preserves_committed_prefix_and_refuses_uncommitted_retry() {
        let w = lease_world().await;
        w.handle
            .input(w.token.clone(), b"0".to_vec())
            .await
            .unwrap();
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        // Native-era commands reach the DO executor only to be refused.
        let refused = w.handle.input(w.token.clone(), b"0".to_vec()).await;
        assert_eq!(
            lease_error(&refused.unwrap_err()),
            Some(LeaseRejectionReason::NotHolder)
        );

        // De-escalation: the lease returns to the DO at the next epoch with
        // no term.
        assert_eq!(
            w.handle
                .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 2, None)
                .await
                .unwrap(),
            2
        );
        // The client retries the refused command; it commits exactly once.
        w.handle
            .input(w.token.clone(), b"0".to_vec())
            .await
            .unwrap();
        assert_eq!(committed_inputs(&w.journal), 2);

        // The journal alone rebuilds DO-held epoch 2: replay is the oracle.
        let restored =
            WorldCore::restore("w", WorldLimits::default(), w.journal.entries()).unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.holder.as_deref(), Some(DO_WRITE_LEASE_HOLDER));
        assert_eq!(lease.epoch, 2);
        assert_eq!(
            lease.transitions().map(|t| t.kind).collect::<Vec<_>>(),
            [
                LeaseTransitionKind::HandedOff,
                LeaseTransitionKind::HandedOff
            ]
        );
        // Live handoffs stamp successive positions (#13711): the
        // first handoff is position 1, the handback position 2, and
        // the successor's watermark is 2.
        assert_eq!(
            lease.transitions().map(|t| t.position).collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(restored.core.lease_position_watermark(), 2);
    }

    #[tokio::test]
    async fn expired_native_lease_fences_the_do_until_recovery_is_journaled() {
        // Walkthrough: lease expiry mid-turn / native crash. Expiry fences
        // the native holder but grants no new owner.
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        w.clock.advance(60_000);
        let refused = w.handle.input(w.token.clone(), b"0".to_vec()).await;
        assert_eq!(
            lease_error(&refused.unwrap_err()),
            Some(LeaseRejectionReason::NotHolder)
        );
        assert_eq!(committed_inputs(&w.journal), 0);

        // Expiry does not grant the DO ownership. Normal expiry recovery
        // (journaled revoke then grant) is a separate slice; a handoff cannot
        // silently turn an expired term into a live one.
        assert!(
            w.handle
                .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 2, None)
                .await
                .is_err()
        );
        let refused_again = w.handle.input(w.token.clone(), b"0".to_vec()).await;
        assert!(refused_again.is_err());
        assert_eq!(committed_inputs(&w.journal), 0);
    }

    #[tokio::test]
    async fn failover_after_expiry_grants_the_caught_up_pool_successor() {
        // Oracle: design #5787 §7 — the holder dies mid-term with a
        // caught-up replica in the pool. The grantor fences first
        // (revoke at e+1), grants the successor at e+2, and the grant's
        // journal position is the barrier the successor resumes at.
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();

        // native-2 has served as a reader at the tip for the full
        // observation window; the grantor's tip for this grant is 2.
        let mut pool = spill_pool::SpillPool::default();
        pool.configure("native-2");
        for cycle in 0..u64::from(spill_pool::READER_OBSERVATION_CYCLES) {
            assert!(pool.observe("native-2", cycle, true, true, true, 2, 2, 0, 0));
        }

        // The term lapses on the authority clock (start 1_000 + 60_000).
        w.clock.advance(60_000);
        let execution = w
            .handle
            .failover_write_lease(pool, 2, 0, 30_000)
            .await
            .unwrap();
        assert_eq!(execution.successor(), "native-2");
        assert_eq!(execution.grant_epoch(), 3);
        assert_eq!(execution.grant_position(), 3);
        assert!(!execution.resume_ready(2));
        assert!(execution.resume_ready(3));

        // The journal carries the fence before the grant, positions
        // continuing the lease chain (handoff 1, revoke 2, grant 3).
        let entries = w.journal.entries();
        assert!(matches!(
            &entries[entries.len() - 2],
            JournalEntry::LeaseRevoked {
                former_holder,
                epoch: 2,
                position: 2,
                ..
            } if former_holder == "native-1"
        ));
        assert!(matches!(
            entries.last(),
            Some(JournalEntry::LeaseGranted {
                holder,
                epoch: 3,
                position: 3,
                ..
            }) if holder == "native-2"
        ));

        // Replay is the oracle: the journal alone rebuilds native-2
        // holding epoch 3, watermark at the grant position, and the
        // dead holder fenced.
        let restored =
            WorldCore::restore("w", WorldLimits::default(), w.journal.entries()).unwrap();
        let lease = restored.core.write_lease();
        assert_eq!(lease.holder.as_deref(), Some("native-2"));
        assert_eq!(lease.epoch, 3);
        assert_eq!(restored.core.lease_position_watermark(), 3);
        assert_eq!(
            lease.check_writer("native-1", 1, 61_000),
            Err(LeaseRejectionReason::NotHolder)
        );

        // The DO executor no longer holds the lease: its commands are
        // refused, exactly as after a handoff.
        let refused = w.handle.input(w.token.clone(), b"0".to_vec()).await;
        assert_eq!(
            lease_error(&refused.unwrap_err()),
            Some(LeaseRejectionReason::NotHolder)
        );
    }

    #[tokio::test]
    async fn failover_measurements_match_the_q2_design_bounds() {
        // Q2 measurements (design #5787 Q2, #7572): the §7
        // holder-death walkthrough on the authority clock with the
        // standing defaults (30 s term, 10 s renewal), measuring
        // detection, grant, and replay-to-ready instead of only the
        // outcome. The clock starts at 1_000; the holder's term is
        // stamped to expire at 31_000 and the holder dies at the
        // stamp — the worst case for detection.
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();

        // native-2 has served as a reader at the tip for the full
        // observation window; the grantor's tip for this grant is 2.
        let mut pool = spill_pool::SpillPool::default();
        pool.configure("native-2");
        for cycle in 0..u64::from(spill_pool::READER_OBSERVATION_CYCLES) {
            assert!(pool.observe("native-2", cycle, true, true, true, 2, 2, 0, 0));
        }

        // Detection: one millisecond before expiry the failover is
        // still refused — no fence can exist before the term lapses,
        // and the refused attempt journals nothing (the successful
        // failover below still lands at positions 2 and 3). At
        // expiry it proceeds. Measured detection latency for a death
        // at the stamp: 31_000 - 1_000 = 30_000 ms — exactly one
        // term, the design's stated worst-case bound.
        w.clock.advance(29_999);
        assert!(
            w.handle
                .failover_write_lease(pool.clone(), 2, 0, 30_000)
                .await
                .is_err()
        );
        w.clock.advance(1);
        let execution = w
            .handle
            .failover_write_lease(pool, 2, 0, 30_000)
            .await
            .unwrap();

        // Grant: the fence and the grant are two consecutive journal
        // appends (positions 2 and 3) — the grant lands one append
        // after the fence, at the same authority instant.
        assert_eq!(execution.entries().len(), 2);
        assert_eq!(execution.grant_position(), 3);
        assert_eq!(execution.grant_epoch(), 3);

        // Replay-to-ready: the caught-up successor, observed at
        // position 2, replays exactly one entry — far less than a
        // cold replay — before it may write: the grant record
        // itself is its barrier.
        assert!(!execution.resume_ready(2));
        assert!(execution.resume_ready(3));

        // The successor's fresh term runs a full 30 s from the
        // failover instant.
        assert_eq!(execution.resulting_lease().expires_at_ms, Some(61_000));
    }

    #[tokio::test]
    async fn spill_copy_dispatch_waits_for_live_successor_replay_and_epoch() {
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        let mut pool = spill_pool::SpillPool::default();
        pool.configure("native-2");
        for cycle in 0..u64::from(spill_pool::READER_OBSERVATION_CYCLES) {
            assert!(pool.observe("native-2", cycle, true, true, true, 2, 2, 0, 0));
        }
        w.clock.advance(60_000);
        let execution = w
            .handle
            .failover_write_lease(pool, 2, 0, 30_000)
            .await
            .unwrap();
        assert_eq!(execution.successor(), "native-2");
        assert_eq!(execution.grant_epoch(), 3);

        let stale_replay = w
            .handle
            .dispatch_spill_copy("native-2", 3, 2, b"journal-copy".to_vec())
            .await;
        assert!(
            stale_replay
                .unwrap_err()
                .to_string()
                .contains("ReplayPending")
        );
        assert_eq!(
            w.handle
                .dispatch_spill_copy(
                    "native-2",
                    3,
                    execution.grant_position(),
                    b"journal-copy".to_vec()
                )
                .await
                .unwrap(),
            b"journal-copy"
        );

        w.handle
            .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 4, None)
            .await
            .unwrap();
        assert!(
            w.handle
                .dispatch_spill_copy(
                    "native-2",
                    3,
                    execution.grant_position(),
                    b"stale".to_vec()
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn spill_controller_dispatches_only_an_authenticated_cursor_at_the_boundary() {
        // Oracle: #7572 successor slice (design #5787 §1 step 4) — the
        // production host/controller path must send journal bytes only
        // for a cursor that (a) authenticates for its exact
        // holder/epoch/position triple, (b) passes the residency gate,
        // and (c) still passes the actor's serialized live
        // holder/epoch/term/replay check at dispatch time. Every
        // refusal sends zero bytes.
        use id_core::residency::{
            Evidence, EvidenceVerifier, Hop, Jurisdiction, Plane, Policy, ResourceId, Scope,
        };
        use spill_controller::{CursorAuthenticator, SpillController, SpillDispatchRefusal};
        use std::cell::RefCell;

        struct MintingAuthenticator;
        fn minted_proof(holder: &str, epoch: u64, position: u64) -> Vec<u8> {
            format!("proof:{holder}:{epoch}:{position}").into_bytes()
        }
        impl CursorAuthenticator for MintingAuthenticator {
            fn authenticate_cursor(
                &self,
                holder: &str,
                epoch: u64,
                replayed_position: u64,
                proof: &[u8],
            ) -> bool {
                proof == minted_proof(holder, epoch, replayed_position).as_slice()
            }
        }
        struct AcceptingVerifier;
        impl EvidenceVerifier for AcceptingVerifier {
            fn signature_valid(&self, _evidence: &Evidence) -> bool {
                true
            }
            fn revoked(&self, _evidence: &Evidence) -> bool {
                false
            }
        }

        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        let mut pool = spill_pool::SpillPool::default();
        pool.configure("native-2");
        for cycle in 0..u64::from(spill_pool::READER_OBSERVATION_CYCLES) {
            assert!(pool.observe("native-2", cycle, true, true, true, 2, 2, 0, 0));
        }
        w.clock.advance(60_000);
        let execution = w
            .handle
            .failover_write_lease(pool, 2, 0, 30_000)
            .await
            .unwrap();
        assert_eq!(execution.successor(), "native-2");
        assert_eq!(execution.grant_epoch(), 3);
        let boundary = execution.grant_position();

        let controller = SpillController::new(&MintingAuthenticator);
        let verifier = AcceptingVerifier;
        let sent: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
        macro_rules! dispatch {
            ($holder:expr, $epoch:expr, $position:expr, $proof:expr, $scope:expr, $hops:expr) => {
                controller
                    .dispatch_presented_copy(
                        &w.handle,
                        $scope,
                        $hops,
                        1_000_000,
                        &verifier,
                        $holder,
                        $epoch,
                        $position,
                        $proof,
                        b"journal-copy".to_vec(),
                        |copy| sent.borrow_mut().push(copy.to_vec()),
                    )
                    .await
            };
        }

        // Forged proof at the boundary position: no cursor, no bytes.
        let forged = dispatch!(
            "native-2",
            3,
            boundary,
            b"forged".as_slice(),
            &Scope::Unscoped,
            &[]
        );
        assert!(matches!(
            forged,
            Err(SpillDispatchRefusal::UnauthenticatedCursor { .. })
        ));
        assert!(sent.borrow().is_empty());

        // Authenticated but stale cursor (below the ownership
        // boundary): the actor's final gate refuses, no bytes.
        let stale = dispatch!(
            "native-2",
            3,
            2,
            &minted_proof("native-2", 3, 2),
            &Scope::Unscoped,
            &[]
        );
        assert!(
            matches!(&stale, Err(SpillDispatchRefusal::Authority(reason)) if reason.contains("ReplayPending")),
            "stale cursor must hit the replay gate, got {stale:?}"
        );
        assert!(sent.borrow().is_empty());

        // Authenticated cursor for the wrong epoch: no bytes.
        let wrong_epoch = dispatch!(
            "native-2",
            2,
            boundary,
            &minted_proof("native-2", 2, boundary),
            &Scope::Unscoped,
            &[]
        );
        assert!(matches!(
            wrong_epoch,
            Err(SpillDispatchRefusal::Authority(_))
        ));
        assert!(sent.borrow().is_empty());

        // Authenticated cursor at the boundary, but the residency gate
        // denies the path (expired host evidence): no bytes.
        let policy = Policy::new(["operator-ca".to_string()])
            .allow(Plane::Processing, &["eu"], &["spill-eu"])
            .trust_evidence_source(Plane::Processing, "provider-control-plane");
        let expired = Evidence {
            issuer: "operator-ca".to_string(),
            resource: ResourceId("spill-eu".to_string()),
            plane: Plane::Processing,
            jurisdiction: Jurisdiction("eu".to_string()),
            source: "provider-control-plane".to_string(),
            expires_at: 999_999,
            serial: "spill-eu-1".to_string(),
            signature: vec![1],
        };
        let hops = [Hop {
            plane: Plane::Processing,
            resource: ResourceId("spill-eu".to_string()),
            evidence: Some(&expired),
        }];
        let residency = dispatch!(
            "native-2",
            3,
            boundary,
            &minted_proof("native-2", 3, boundary),
            &Scope::Scoped(policy),
            &hops
        );
        assert!(matches!(
            residency,
            Err(SpillDispatchRefusal::Residency(_))
        ));
        assert!(sent.borrow().is_empty());

        // The cursor at the current ownership boundary dispatches:
        // exactly one send, with the actor-released bytes.
        let dispatched = dispatch!(
            "native-2",
            3,
            boundary,
            &minted_proof("native-2", 3, boundary),
            &Scope::Unscoped,
            &[]
        );
        assert_eq!(dispatched, Ok(()));
        assert_eq!(sent.borrow().as_slice(), [b"journal-copy".to_vec()]);

        // After handback, the previously valid epoch-3 cursor is stale
        // at the actor's serialized gate: no further bytes.
        w.handle
            .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 4, None)
            .await
            .unwrap();
        let after_handback = dispatch!(
            "native-2",
            3,
            boundary,
            &minted_proof("native-2", 3, boundary),
            &Scope::Unscoped,
            &[]
        );
        assert!(matches!(
            after_handback,
            Err(SpillDispatchRefusal::Authority(_))
        ));
        assert_eq!(sent.borrow().len(), 1);
    }

    #[tokio::test]
    async fn failover_with_no_eligible_successor_recovers_to_the_do() {
        // Oracle: design #5787 §7 — no eligible successor: the DO
        // grants to itself rather than to a lagging replica, and the
        // world resumes under its authority.
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        w.clock.advance(60_000);
        let execution = w
            .handle
            .failover_write_lease(spill_pool::SpillPool::default(), 0, 0, 30_000)
            .await
            .unwrap();
        assert_eq!(execution.successor(), DO_WRITE_LEASE_HOLDER);
        w.handle
            .input(w.token.clone(), b"0".to_vec())
            .await
            .unwrap();
        assert_eq!(committed_inputs(&w.journal), 1);
    }

    #[tokio::test]
    async fn stale_handoff_proposals_are_refused_and_journal_nothing() {
        // A host working from a stale lease view proposes an epoch the
        // authority has already passed.
        let w = lease_world().await;
        w.handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap();
        let before = w.journal.entries().len();
        // Stale: proposes epoch 1 again, and a handback that skips epochs.
        assert!(
            w.handle
                .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 1, None)
                .await
                .is_err()
        );
        assert!(
            w.handle
                .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 3, None)
                .await
                .is_err()
        );
        // A native lease needs a future term; the DO lease must be untermed.
        assert!(
            w.handle
                .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 2, Some(99_000))
                .await
                .is_err()
        );
        assert_eq!(w.journal.entries().len(), before);

        // The correct, caught-up proposal still succeeds afterwards.
        w.handle
            .handoff_write_lease(DO_WRITE_LEASE_HOLDER, 2, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn failed_handoff_append_leaves_the_do_holding_but_read_only() {
        // An append failure at the boundary must not move the lease; the
        // world fails closed.
        struct FailingJournal;
        impl WorldJournal for FailingJournal {
            fn append(&mut self, _entry: &JournalEntry) -> Result<()> {
                bail!("injected append failure")
            }
        }
        let clock = Arc::new(ManualClock::new(1_000));
        let core = WorldCore::new_with_clock("w", WorldLimits::default(), clock).unwrap();
        let handle =
            WorldHandle::spawn_durable(core, Box::new(Mock::new()), Some(Box::new(FailingJournal)));
        assert!(
            handle
                .handoff_write_lease("native-1", 1, Some(31_000))
                .await
                .is_err()
        );
        // No holder change was acknowledged: the world is read-only, not
        // handed to a second executor.
        let again = handle
            .handoff_write_lease("native-1", 1, Some(31_000))
            .await
            .unwrap_err();
        assert!(format!("{again:#}").contains("storage failed"), "{again:#}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod account_tests {
    use super::*;
    use crate::directory::{Actor, Tier};
    use crate::world::*;

    fn core() -> WorldCore {
        WorldCore::new("w", WorldLimits::default()).unwrap()
    }

    fn account(core: &mut WorldCore, name: &str) -> (String, String) {
        let (credential, _) = core.directory_mut().sign_up(name).unwrap();
        let id = core.directory().account_for(&credential).unwrap();
        (id, credential)
    }

    #[tokio::test]
    async fn a_friend_delegation_through_the_handle_is_held_to_the_friendship() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (cy, _) = account(&mut core, "Cy");
        core.directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, ann_token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let handle = WorldHandle::spawn(core);

        let (_, bo_token) = handle
            .delegate_to_friend(
                ann_token.clone(),
                bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo",
            )
            .await
            .unwrap();
        handle.chat(bo_token, "hi").await.unwrap();
        assert!(
            handle
                .delegate_to_friend(
                    ann_token,
                    cy,
                    WorldScopes::GUEST,
                    CapabilityBounds::default(),
                    "Cy"
                )
                .await
                .is_err(),
            "only friends may be delegated to"
        );
    }

    #[tokio::test]
    async fn a_bounded_issue_stops_after_its_uses() {
        let handle = WorldHandle::spawn(core());
        let bounds = CapabilityBounds {
            expires_at: None,
            uses: Some(1),
        };
        let (_, token) = handle
            .issue_bounded("Ann", WorldScopes::GUEST, bounds)
            .await
            .unwrap();
        handle.chat(token.clone(), "once").await.unwrap();
        assert!(handle.chat(token, "twice").await.is_err());
    }

    fn befriend(core: &mut WorldCore, a: &str, a_credential: &str, b: &str, b_credential: &str) {
        let world = core.world_id().to_owned();
        core.directory_mut()
            .request_friend(a_credential, b, &world, 1)
            .unwrap();
        core.directory_mut()
            .accept_friend(b_credential, a, &world, 2)
            .unwrap();
    }

    #[test]
    fn a_ceiling_that_allows_chat_allows_reading_it() {
        let mut core = core();
        let (_, ann_credential) = account(&mut core, "Ann");
        let (_, guest) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (_, reader) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::JOIN.union(WorldScopes::HISTORY),
                CapabilityBounds::default(),
                "Ann reads",
            )
            .unwrap();
        core.chat(&guest, "hello").unwrap();
        assert_eq!(core.snapshot(&reader).unwrap().events.len(), 1);
        assert!(core.chat(&reader, "hi").is_err());
    }

    #[test]
    fn an_account_capability_is_held_to_its_ceiling() {
        let mut core = core();
        let (_, ann_credential) = account(&mut core, "Ann");
        assert!(
            core.issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann"
            )
            .is_err(),
            "a registered account cannot be issued delegate scope"
        );
        assert!(
            core.issue_for_account(
                "acct.forged.00",
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann"
            )
            .is_err()
        );
        let (_, token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        core.chat(&token, "hello").unwrap();
        core.directory_mut()
            .set_tier(Actor::Server, Tier::Registered, WorldScopes::JOIN)
            .unwrap();
        assert!(
            core.chat(&token, "again").is_err(),
            "a narrowed tier narrows the capability"
        );
        assert!(core.snapshot(&token).is_ok());
    }

    #[test]
    fn an_account_capability_serializes_its_subject_but_no_secret() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (participant, _) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let entry = core.issued_entry(participant.id).unwrap();
        let text = serde_json::to_string(&entry).unwrap();
        assert!(text.contains(&ann));
        let secret = ann_credential.rsplit('.').next().unwrap();
        assert!(!text.contains(secret));
        let back: JournalEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn a_delegation_to_a_friend_is_held_to_the_friends_ceiling_and_ends_with_the_friendship() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (cy, _) = account(&mut core, "Cy");
        core.directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, ann_token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();

        assert!(
            core.delegate_to_friend(
                &ann_token,
                &bo,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Bo"
            )
            .is_err(),
            "bo's ceiling does not include delegate"
        );
        assert!(
            core.delegate_to_friend(
                &ann_token,
                &cy,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Cy"
            )
            .is_err(),
            "only friends may be delegated to"
        );
        let (_, bo_token) = core
            .delegate_to_friend(
                &ann_token,
                &bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        core.chat(&bo_token, "hi").unwrap();
        assert!(
            core.attenuate(
                &bo_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Di"
            )
            .is_err(),
            "bo's delegation carries no delegate scope"
        );

        core.directory_mut()
            .remove_friend(&ann_credential, &bo)
            .unwrap();
        assert!(
            core.snapshot(&bo_token).is_err(),
            "the delegation ends with the friendship"
        );
        assert!(core.snapshot(&ann_token).is_ok());
    }

    #[test]
    fn only_an_account_capability_may_delegate_to_a_friend() {
        let mut core = core();
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (ann, ann_credential) = account(&mut core, "Ann");
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, anonymous) = core
            .issue_capability("Guest", WorldScopes::ALL, CapabilityBounds::default())
            .unwrap();
        assert!(
            core.delegate_to_friend(
                &anonymous,
                &bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo"
            )
            .is_err()
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod isolation_tests {
    use super::*;
    use crate::directory::{Actor, Level, Member, Refusal};
    use crate::world::*;

    struct Staff {
        core: WorldCore,
        member: String,
        writer: String,
        manager: String,
        moderator: String,
        admin: String,
        stranger: String,
    }

    fn account(core: &mut WorldCore, name: &str) -> String {
        let (credential, _) = core.directory_mut().sign_up(name).unwrap();
        core.directory().account_for(&credential).unwrap()
    }

    fn seat(core: &mut WorldCore, group: u64, name: &str, level: Level) -> String {
        let id = account(core, name);
        core.directory_mut()
            .set_member(
                Actor::Server,
                group,
                Member::Account { id: id.clone() },
                Some(level),
            )
            .unwrap();
        id
    }

    /// One group with a holder of each level, plus an account outside it.
    fn staffed() -> Staff {
        let mut core = WorldCore::new("staffed", WorldLimits::default()).unwrap();
        let founder = account(&mut core, "Founder");
        let (group, _) = core
            .directory_mut()
            .create_group(&founder, "staff", "", WorldScopes::GUEST)
            .unwrap();
        Staff {
            member: seat(&mut core, group, "Member", Level::Access),
            writer: seat(&mut core, group, "Writer", Level::Write),
            manager: seat(&mut core, group, "Manager", Level::Manage),
            moderator: seat(&mut core, group, "Moderator", Level::Moderator),
            admin: seat(&mut core, group, "Admin", Level::Admin),
            stranger: account(&mut core, "Stranger"),
            core,
        }
    }

    #[tokio::test]
    async fn worlds_start_isolated() {
        assert_eq!(Isolation::default(), Isolation::Isolated);
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        assert_eq!(handle.isolation().await.unwrap(), Isolation::Isolated);
    }

    #[tokio::test]
    async fn only_a_moderator_or_an_admin_sets_isolation() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_isolation(Authority::Signed(who.clone()), Isolation::Unisolated)
                .await
                .unwrap_err();
            assert!(
                matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_))),
                "{who} must not change isolation"
            );
        }
        assert_eq!(handle.isolation().await.unwrap(), Isolation::Isolated);

        assert_eq!(
            handle
                .set_isolation(
                    Authority::Signed(staff.moderator.clone()),
                    Isolation::Unisolated
                )
                .await
                .unwrap(),
            Isolation::Unisolated
        );
        assert_eq!(
            handle
                .set_isolation(Authority::Signed(staff.admin.clone()), Isolation::Isolated)
                .await
                .unwrap(),
            Isolation::Isolated
        );
        assert_eq!(
            handle
                .set_isolation(Authority::Admin, Isolation::Unisolated)
                .await
                .unwrap(),
            Isolation::Unisolated
        );
    }

    #[test]
    fn a_checkpoint_keeps_the_isolation() {
        let mut core = WorldCore::new("iso", WorldLimits::default()).unwrap();
        core.isolation = Isolation::Unisolated;
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Isolation {
                isolation: Isolation::Unisolated
            }
        )));
    }

    fn forbidden(error: &anyhow::Error) -> bool {
        matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_)))
    }

    #[tokio::test]
    async fn only_a_moderator_or_an_admin_sets_cross_world_write() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_cross_world_write(Authority::Signed(who.clone()), true)
                .await
                .unwrap_err();
            assert!(
                forbidden(&error),
                "{who} must not change cross-world writes"
            );
        }
        assert!(!handle.policy().await.unwrap().cross_world_write);

        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), true)
            .await
            .unwrap();
        assert!(policy.cross_world_write && !policy.cross_world_forced);
        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write);
    }

    #[tokio::test]
    async fn only_the_admin_forces_cross_world_writes() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        for who in [&staff.moderator, &staff.admin] {
            let error = handle
                .force_cross_world_write(Authority::Signed(who.clone()))
                .await
                .unwrap_err();
            assert!(forbidden(&error), "{who} must not force cross-world writes");
        }
        assert!(!handle.policy().await.unwrap().cross_world_forced);
    }

    #[tokio::test]
    async fn an_isolated_world_cannot_be_forced() {
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        let error = handle
            .force_cross_world_write(Authority::Admin)
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        assert!(error.to_string().contains("isolated"), "{error}");
        assert!(!handle.policy().await.unwrap().cross_world_forced);
    }

    #[tokio::test]
    async fn a_forced_write_stays_on_against_a_moderator() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        handle
            .force_cross_world_write(Authority::Admin)
            .await
            .unwrap();

        let error = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        let policy = handle.policy().await.unwrap();
        assert!(policy.cross_world_write && policy.cross_world_forced);

        let policy = handle
            .set_cross_world_write(Authority::Admin, false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write && !policy.cross_world_forced);
    }

    #[tokio::test]
    async fn a_moderator_turns_off_a_write_that_was_not_forced() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), true)
            .await
            .unwrap();
        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write);
    }

    #[tokio::test]
    async fn file_scope_is_moderated_and_the_override_is_admin_only() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        assert_eq!(
            handle.policy().await.unwrap(),
            WorldPolicy {
                isolation: Isolation::Isolated,
                cross_world_write: false,
                cross_world_forced: false,
                file_scope: FileScope::Confined,
                file_write_override: false,
            }
        );
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_file_scope(
                    Authority::Signed(who.clone()),
                    FileScope::Unrestricted,
                    None,
                )
                .await
                .unwrap_err();
            assert!(forbidden(&error), "{who} must not change the file scope");
        }

        let error = handle
            .set_file_scope(
                Authority::Signed(staff.moderator.clone()),
                FileScope::Unrestricted,
                Some(true),
            )
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        assert_eq!(
            handle.policy().await.unwrap().file_scope,
            FileScope::Confined
        );

        let policy = handle
            .set_file_scope(
                Authority::Signed(staff.moderator.clone()),
                FileScope::Unrestricted,
                None,
            )
            .await
            .unwrap();
        assert_eq!(policy.file_scope, FileScope::Unrestricted);
        assert!(!policy.file_write_override);

        let policy = handle
            .set_file_scope(Authority::Admin, FileScope::Unrestricted, Some(true))
            .await
            .unwrap();
        assert!(policy.file_write_override);
    }

    #[test]
    fn file_access_follows_isolation_scope_and_override() {
        use FileMode::{Read, Write};
        use FileScope::{Confined, Unrestricted};
        use Isolation::{Isolated, Unisolated};
        let policy = |isolation, file_scope, file_write_override| WorldPolicy {
            isolation,
            cross_world_write: false,
            cross_world_forced: false,
            file_scope,
            file_write_override,
        };
        // (policy, mode, inside own root, allowed)
        let cases = [
            (policy(Unisolated, Confined, false), Read, true, true),
            (policy(Unisolated, Confined, false), Read, false, false),
            (policy(Unisolated, Confined, false), Write, true, true),
            (policy(Unisolated, Confined, false), Write, false, false),
            (policy(Unisolated, Unrestricted, false), Read, false, true),
            (policy(Unisolated, Unrestricted, false), Write, false, true),
            (policy(Isolated, Confined, false), Read, true, true),
            (policy(Isolated, Confined, false), Read, false, false),
            (policy(Isolated, Confined, false), Write, true, false),
            (policy(Isolated, Unrestricted, false), Read, false, true),
            (policy(Isolated, Unrestricted, false), Write, false, false),
            (policy(Isolated, Unrestricted, true), Write, false, true),
            (policy(Isolated, Confined, true), Write, true, true),
            (policy(Isolated, Confined, true), Write, false, false),
        ];
        for (policy, mode, inside_root, expected) in cases {
            assert_eq!(
                policy.file_allowed(mode, inside_root),
                expected,
                "{policy:?} {mode:?} inside root {inside_root}"
            );
        }
    }

    #[test]
    fn a_default_world_reads_only_inside_its_root() {
        let policy = WorldCore::new("w", WorldLimits::default())
            .unwrap()
            .policy();
        assert_eq!(policy.file_scope, FileScope::Confined);
        assert!(!policy.file_write_override);
        assert!(policy.file_allowed(FileMode::Read, true));
        assert!(!policy.file_allowed(FileMode::Read, false));
        assert!(!policy.file_allowed(FileMode::Write, true));
    }

    #[test]
    fn a_checkpoint_keeps_the_sharing_settings() {
        for (cross_world_write, cross_world_forced) in [(true, false), (true, true), (false, false)]
        {
            let mut core = WorldCore::new("share", WorldLimits::default()).unwrap();
            core.isolation = Isolation::Unisolated;
            core.cross_world_write = cross_world_write;
            core.cross_world_forced = cross_world_forced;
            core.file_scope = FileScope::Unrestricted;
            core.file_write_override = true;
            core.active_module_hash = Some("h".to_owned());
            let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
            let restored = WorldCore::restore("share", WorldLimits::default(), entries).unwrap();
            assert_eq!(restored.core.policy(), core.policy());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod schedule_tests {
    use super::*;
    use crate::world::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn world(retained_events: usize) -> WorldCore {
        WorldCore::new(
            "test-world",
            WorldLimits {
                retained_events,
                ..WorldLimits::default()
            },
        )
        .unwrap()
    }

    /// A program for the schedule tests. Each input is a request document, and
    /// the program asks for its requests until each one's result arrives. Its
    /// records carry every schedule fire and every result, so a test reads them
    /// through `WorldHandle::records`.
    #[derive(Default)]
    struct Scheduler {
        requests: Vec<serde_json::Value>,
        answered: BTreeSet<String>,
        results: BTreeMap<String, serde_json::Value>,
        fires: Vec<serde_json::Value>,
        refuse_fires: bool,
    }

    impl WorldProgram for Scheduler {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            match &event.kind {
                WorldEventKind::Input(bytes) => {
                    let document: serde_json::Value = serde_json::from_slice(bytes)?;
                    self.requests = document["requests"].as_array().cloned().unwrap_or_default();
                }
                WorldEventKind::EntityInput { input, .. } => {
                    let document: serde_json::Value = serde_json::from_slice(input)?;
                    self.requests = document["requests"].as_array().cloned().unwrap_or_default();
                }
                WorldEventKind::System { event: text } => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
                        return Ok(());
                    };
                    match value["cap"].as_str() {
                        Some("result") => {
                            if let Some(id) = value["id"].as_str() {
                                self.answered.insert(id.to_owned());
                                self.results.insert(id.to_owned(), value.clone());
                            }
                        }
                        Some("schedule") => {
                            if self.refuse_fires {
                                bail!("this program refuses schedule fires");
                            }
                            self.fires.push(value.clone());
                        }
                        _ => {}
                    }
                }
                WorldEventKind::Chat(_)
                | WorldEventKind::ProgramInstalled { .. }
                | WorldEventKind::Notification { .. } => {}
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let pending: Vec<&serde_json::Value> = self
                .requests
                .iter()
                .filter(|request| {
                    !request["id"]
                        .as_str()
                        .is_some_and(|id| self.answered.contains(id))
                })
                .collect();
            Ok(Some(
                serde_json::json!({"v": 1, "requests": pending}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"fired": self.fires, "results": self.results}).to_string(),
            ))
        }
    }

    /// A journal kept in memory, so a test can restore from what was written.
    #[derive(Clone, Default)]
    struct Recorded(Arc<std::sync::Mutex<Vec<JournalEntry>>>);

    impl Recorded {
        /// A journal that starts with the header a real world writes first.
        fn for_world(world_id: &str) -> Self {
            let journal = Self::default();
            journal.0.lock().unwrap().push(JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            });
            journal
        }

        fn entries(&self) -> Vec<JournalEntry> {
            self.0.lock().unwrap().clone()
        }
    }

    impl WorldJournal for Recorded {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            super::tests::push_bumped(&mut self.0.lock().unwrap(), entry);
            Ok(())
        }
    }

    /// The manual clock's start time, in unix milliseconds.
    const EPOCH_MS: u64 = 1_000_000_000_000;

    /// A journaled actor running a `Scheduler` on a manual clock, with one
    /// participant and the given grants. Nothing is spawned: the test drives
    /// [`WorldActor::handle`], [`WorldActor::after_change`] and
    /// [`WorldActor::fire_schedules`] directly, advancing the clock by hand.
    fn scheduling_actor(
        id: &str,
        grants: &[&str],
        refuse_fires: bool,
    ) -> (WorldActor, Arc<ManualClock>, JoinCapability, Recorded) {
        let journal = Recorded::for_world(id);
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core =
            WorldCore::new_with_clock(id, WorldLimits::default(), clock.clone()).unwrap();
        core.ledger
            .set_granted(grants.iter().map(|grant| (*grant).to_owned()).collect());
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        // Production issuance goes through the actor, which journals the
        // `Issued` entry as it mints (`record_issued`). Minting directly on
        // the core must record the same entry: without it the journal uses
        // the participant before it was issued -- a state no live path can
        // write, and restore refuses it (#2894).
        let issued = core.issued_entry(ann.id).unwrap();
        super::tests::push_bumped(&mut journal.0.lock().unwrap(), &issued);
        let program = Scheduler {
            refuse_fires,
            ..Scheduler::default()
        };
        let actor = WorldActor::for_test(core, Box::new(program), Some(Box::new(journal.clone())));
        (actor, clock, token, journal)
    }

    /// Drive one input document through the actor exactly as its run loop
    /// would: handle, settle the program's wants, then clear `acting`.
    async fn ask(actor: &mut WorldActor, token: &JoinCapability, document: serde_json::Value) {
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::Input {
                token: token.clone(),
                input: document.to_string().into_bytes(),
                session_id: None,
                device_input: None,
                signed_move: None,
                reply,
            })
            .await;
        actor.after_change().await;
        actor.acting = None;
        let _ = response.await.unwrap().unwrap();
    }

    /// A journal that records how the actor persisted - plain appends
    /// vs batches with their sizes - so a group-commit test observes the
    /// batching itself, not just its effects (#8239).
    #[derive(Clone, Default)]
    struct BatchJournal {
        entries: Arc<std::sync::Mutex<Vec<JournalEntry>>>,
        appends: Arc<std::sync::Mutex<usize>>,
        batches: Arc<std::sync::Mutex<Vec<usize>>>,
    }

    impl BatchJournal {
        fn for_world(world_id: &str) -> Self {
            let journal = Self::default();
            journal.entries.lock().unwrap().push(JournalEntry::Created {
                world_id: world_id.to_owned(),
                version: JOURNAL_VERSION,
            });
            journal
        }

        fn committed(&self) -> Vec<JournalEntry> {
            self.entries
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::Committed { .. }))
                .cloned()
                .collect()
        }
    }

    impl WorldJournal for BatchJournal {
        fn append(&mut self, entry: &JournalEntry) -> Result<()> {
            *self.appends.lock().unwrap() += 1;
            self.entries.lock().unwrap().push(entry.clone());
            Ok(())
        }

        fn append_batch(&mut self, entries: &[JournalEntry]) -> Result<()> {
            self.batches.lock().unwrap().push(entries.len());
            self.entries.lock().unwrap().extend(entries.iter().cloned());
            Ok(())
        }
    }

    /// A program that refuses the input `boom` and accepts everything else.
    struct BatchProgram;

    impl WorldProgram for BatchProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(input) = &event.kind {
                ensure!(input != b"boom", "the program refuses this input");
            }
            Ok(())
        }
    }

    fn batch_actor(
        id: &str,
        bounds: CapabilityBounds,
    ) -> (WorldActor, JoinCapability, BatchJournal) {
        batch_actor_with(id, bounds, Box::new(BatchProgram))
    }

    fn batch_actor_with(
        id: &str,
        bounds: CapabilityBounds,
        program: Box<dyn WorldProgram>,
    ) -> (WorldActor, JoinCapability, BatchJournal) {
        let journal = BatchJournal::for_world(id);
        let mut core = WorldCore::new(id, WorldLimits::default()).unwrap();
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, bounds)
            .unwrap();
        let issued = core.issued_entry(ann.id).unwrap();
        journal.entries.lock().unwrap().push(issued);
        let actor = WorldActor::for_test(core, program, Some(Box::new(journal.clone())));
        (actor, token, journal)
    }

    /// Drive one drained batch through the actor's batch handler and
    /// collect every reply, in order.
    async fn batch_replies(
        actor: &mut WorldActor,
        token: &JoinCapability,
        inputs: &[&[u8]],
    ) -> Vec<Result<WorldEvent>> {
        let mut commands = Vec::new();
        let mut responses = Vec::new();
        for input in inputs {
            let (reply, response) = tokio::sync::oneshot::channel();
            commands.push(WorldCommand::Input {
                token: token.clone(),
                input: input.to_vec(),
                session_id: None,
                device_input: None,
                signed_move: None,
                reply,
            });
            responses.push(response);
        }
        actor.handle_input_batch(commands).await;
        let mut results = Vec::new();
        for response in responses {
            results.push(response.await.unwrap());
        }
        results
    }

    // #8239 oracle: a batch of N inputs commits N events in order, with
    // consecutive sequences, persisted as ONE batch append and no plain
    // appends - one sync covers the whole group.
    #[tokio::test]
    async fn input_batch_commits_n_events_with_one_batch_append() {
        let (mut actor, token, journal) = batch_actor("batch-ok", CapabilityBounds::default());
        let results = batch_replies(
            &mut actor,
            &token,
            &[b"one", b"two", b"three", b"four", b"five"],
        )
        .await;
        let sequences: Vec<u64> = results
            .iter()
            .map(|result| result.as_ref().unwrap().sequence)
            .collect();
        assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
        assert_eq!(journal.committed().len(), 5);
        assert_eq!(*journal.batches.lock().unwrap(), vec![5]);
        assert_eq!(*journal.appends.lock().unwrap(), 0);
    }

    // #8239 oracle: staging stops at the first program rejection - the
    // staged prefix commits, the rejected input is answered with the
    // rejection the single path produces, and the input after it is
    // routed through the single path, where the now-unhealthy program
    // refuses it as unavailable. Nothing after the prefix is journaled.
    #[tokio::test]
    async fn input_batch_stops_at_program_rejection() {
        let (mut actor, token, journal) = batch_actor("batch-boom", CapabilityBounds::default());
        let results = batch_replies(&mut actor, &token, &[b"one", b"boom", b"two"]).await;
        assert!(results[0].is_ok(), "{:?}", results[0]);
        let rejected = format!("{:#}", results[1].as_ref().unwrap_err());
        assert!(
            rejected.contains("world program rejected input"),
            "{rejected}"
        );
        let unavailable = format!("{:#}", results[2].as_ref().unwrap_err());
        assert!(unavailable.contains("unavailable"), "{unavailable}");
        assert_eq!(journal.committed().len(), 1);
        assert_eq!(*journal.batches.lock().unwrap(), vec![1]);
        assert!(!actor.program_healthy);
    }

    /// A program that refuses the input `limited` at the account's
    /// fuel rate and accepts everything else (#1067).
    struct FuelLimitedBatchProgram;

    impl WorldProgram for FuelLimitedBatchProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::Input(input) = &event.kind {
                if input == b"limited" {
                    return Err(anyhow::Error::new(
                        id_caps::account_quota::FuelRateRefusal {
                            limit_per_second: 10_000,
                            charged_in_window: 10_000,
                            requested_fuel: 10_000,
                        },
                    ));
                }
            }
            Ok(())
        }
    }

    // #1067 oracle: a fuel-rate refusal inside a batch is recoverable -
    // the staged prefix commits, the refused input is answered with the
    // refusal, and the input after it goes through the single path, where
    // the still-healthy program accepts it instead of refusing it as
    // unavailable.
    #[tokio::test]
    async fn a_fuel_rate_refusal_in_a_batch_keeps_the_actor_healthy() {
        let (mut actor, token, journal) = batch_actor_with(
            "batch-fuel-rate",
            CapabilityBounds::default(),
            Box::new(FuelLimitedBatchProgram),
        );
        let results = batch_replies(&mut actor, &token, &[b"one", b"limited", b"two"]).await;
        assert!(results[0].is_ok(), "{:?}", results[0]);
        let refused = format!("{:#}", results[1].as_ref().unwrap_err());
        assert!(refused.contains("fuel rate"), "{refused}");
        assert!(
            !refused.contains("unavailable after a previous failure"),
            "{refused}"
        );
        assert!(results[2].is_ok(), "{:?}", results[2]);
        assert!(actor.program_healthy);
        assert_eq!(journal.committed().len(), 2);
        assert_eq!(*journal.batches.lock().unwrap(), vec![1]);
    }

    /// A program that refuses host (system) events at the account's
    /// fuel rate and accepts everything else (#1067).
    struct FuelLimitedSystemProgram;

    impl WorldProgram for FuelLimitedSystemProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { .. } = &event.kind {
                return Err(anyhow::Error::new(
                    id_caps::account_quota::FuelRateRefusal {
                        limit_per_second: 10_000,
                        charged_in_window: 10_000,
                        requested_fuel: 10_000,
                    },
                ));
            }
            Ok(())
        }
    }

    // #1067 oracle: a fuel-rate refusal of a host event is
    // recoverable — a refused schedule fire leaves the schedule
    // pending and the program healthy, and the next participant
    // input still succeeds.
    #[tokio::test]
    async fn a_fuel_rate_refusal_of_a_host_event_keeps_the_actor_healthy() {
        let journal = Recorded::for_world("system-fuel-rate");
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core = WorldCore::new_with_clock(
            "system-fuel-rate",
            WorldLimits::default(),
            clock.clone(),
        )
        .unwrap();
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let issued = core.issued_entry(ann.id).unwrap();
        super::tests::push_bumped(&mut journal.0.lock().unwrap(), &issued);
        let mut actor = WorldActor::for_test(
            core,
            Box::new(FuelLimitedSystemProgram),
            Some(Box::new(journal.clone())),
        );
        actor.core.schedules.insert("tick".to_owned(), PendingSchedule::one_shot(EPOCH_MS));
        actor.fire_schedules().await;
        assert!(actor.program_healthy);
        assert!(actor.core.schedules.contains_key("tick"));
        let results = batch_replies(&mut actor, &token, &[b"after"]).await;
        assert!(results[0].is_ok(), "{:?}", results[0]);
    }

    /// A program for the #1067 fuel-window tests: it asks for one
    /// `time.now` result, records every result and schedule fire it
    /// receives, and refuses system events at the account's fuel rate
    /// while the manual clock is before `refuse_until_ms` — so the
    /// test drives the rollover by advancing the clock, not the wall.
    /// The attempt counters are shared with the test, so a refused
    /// delivery that must not have been attempted is observable.
    struct FuelWindowProgram {
        clock: Arc<ManualClock>,
        refuse_until_ms: u64,
        requests: Vec<serde_json::Value>,
        answered: BTreeSet<String>,
        results: BTreeMap<String, serde_json::Value>,
        result_deliveries: Arc<AtomicUsize>,
        fire_attempts: Arc<AtomicUsize>,
        fires: Vec<serde_json::Value>,
    }

    impl FuelWindowProgram {
        fn fuel_refusal() -> anyhow::Error {
            anyhow::Error::new(id_caps::account_quota::FuelRateRefusal {
                limit_per_second: 10_000,
                charged_in_window: 10_000,
                requested_fuel: 10_000,
            })
        }
    }

    impl WorldProgram for FuelWindowProgram {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            match &event.kind {
                WorldEventKind::Input(bytes) => {
                    let document: serde_json::Value = serde_json::from_slice(bytes)?;
                    self.requests = document["requests"].as_array().cloned().unwrap_or_default();
                }
                WorldEventKind::EntityInput { input, .. } => {
                    let document: serde_json::Value = serde_json::from_slice(input)?;
                    self.requests = document["requests"].as_array().cloned().unwrap_or_default();
                }
                WorldEventKind::System { event: text } => {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
                        return Ok(());
                    };
                    let refused = self.clock.now_ms() < self.refuse_until_ms;
                    match value["cap"].as_str() {
                        Some("result") => {
                            if refused {
                                return Err(Self::fuel_refusal());
                            }
                            if let Some(id) = value["id"].as_str() {
                                self.answered.insert(id.to_owned());
                                self.results.insert(id.to_owned(), value.clone());
                                self.result_deliveries.fetch_add(1, Ordering::SeqCst);
                            }
                        }
                        Some("schedule") => {
                            self.fire_attempts.fetch_add(1, Ordering::SeqCst);
                            if refused {
                                return Err(Self::fuel_refusal());
                            }
                            self.fires.push(value.clone());
                        }
                        _ => {}
                    }
                }
                WorldEventKind::Chat(_)
                | WorldEventKind::ProgramInstalled { .. }
                | WorldEventKind::Notification { .. } => {}
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            let pending: Vec<&serde_json::Value> = self
                .requests
                .iter()
                .filter(|request| {
                    !request["id"]
                        .as_str()
                        .is_some_and(|id| self.answered.contains(id))
                })
                .collect();
            Ok(Some(
                serde_json::json!({"v": 1, "requests": pending}).to_string(),
            ))
        }

        fn records(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({"fired": self.fires, "results": self.results}).to_string(),
            ))
        }
    }

    /// A `FuelWindowProgram` actor on a manual clock, with `time.now`
    /// granted and one participant. Mirrors the
    /// `FuelLimitedSystemProgram` setup above; the test drives
    /// [`WorldActor::handle`], [`WorldActor::after_change`] and
    /// [`WorldActor::fire_schedules`] directly.
    #[allow(clippy::type_complexity)]
    fn fuel_window_actor(
        id: &str,
        refuse_until_ms: u64,
    ) -> (
        WorldActor,
        Arc<ManualClock>,
        JoinCapability,
        Recorded,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
    ) {
        let journal = Recorded::for_world(id);
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core =
            WorldCore::new_with_clock(id, WorldLimits::default(), clock.clone()).unwrap();
        core.ledger
            .set_granted(BTreeSet::from(["time.now".to_owned()]));
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let issued = core.issued_entry(ann.id).unwrap();
        super::tests::push_bumped(&mut journal.0.lock().unwrap(), &issued);
        let result_deliveries = Arc::new(AtomicUsize::new(0));
        let fire_attempts = Arc::new(AtomicUsize::new(0));
        let program = FuelWindowProgram {
            clock: clock.clone(),
            refuse_until_ms,
            requests: Vec::new(),
            answered: BTreeSet::new(),
            results: BTreeMap::new(),
            result_deliveries: result_deliveries.clone(),
            fire_attempts: fire_attempts.clone(),
            fires: Vec::new(),
        };
        let actor = WorldActor::for_test(core, Box::new(program), Some(Box::new(journal.clone())));
        (
            actor,
            clock,
            token,
            journal,
            result_deliveries,
            fire_attempts,
        )
    }

    /// How many capability results for request `id` the journal
    /// delivered to the program: the exactly-once oracle for a
    /// fuel-refused result.
    fn journaled_results(journal: &Recorded, id: &str) -> usize {
        journal
            .entries()
            .into_iter()
            .filter(|entry| match entry {
                JournalEntry::System { event } => match &event.kind {
                    WorldEventKind::System { event: text } => {
                        serde_json::from_str::<serde_json::Value>(text)
                            .is_ok_and(|value| {
                                value["cap"] == "result" && value["id"] == id
                            })
                    }
                    _ => false,
                },
                _ => false,
            })
            .count()
    }

    // #1067 oracle (Codex review on #12557): a capability result
    // whose delivery is refused at the fuel rate is queued, not
    // lost — the side effect already ran, so the result must not be
    // re-executed — and is delivered exactly once after the window
    // rolls, without another attempt before then.
    #[tokio::test]
    async fn a_fuel_refused_capability_result_is_queued_and_retried_after_rollover() {
        let (mut actor, clock, token, journal, result_deliveries, _) =
            fuel_window_actor("fuel-result", EPOCH_MS + 1_000);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "now-1", "cap": "time.now"},
            ]}),
        )
        .await;
        // The request executed — its id is handled — but the program
        // never saw the result: it waits in the queue instead.
        assert!(actor.program_healthy);
        assert!(actor.handled_requests.contains("now-1"));
        assert_eq!(actor.pending_results.len(), 1);
        assert_eq!(result_deliveries.load(Ordering::SeqCst), 0);
        assert_eq!(journaled_results(&journal, "now-1"), 0);
        // No retry before the window rolls, however often the actor
        // settles: the queue is not a hot loop.
        actor.after_change().await;
        clock.advance(500);
        actor.after_change().await;
        assert_eq!(actor.pending_results.len(), 1);
        assert_eq!(result_deliveries.load(Ordering::SeqCst), 0);
        // Past the rollover the queued result is delivered, exactly
        // once, and the program answers with it — no re-execution, no
        // duplicate on a further settle.
        clock.advance(600);
        actor.after_change().await;
        assert_eq!(result_deliveries.load(Ordering::SeqCst), 1);
        assert!(actor.pending_results.is_empty());
        assert!(actor.fuel_retry_not_before.is_none());
        let result = schedule_result(&actor, "now-1").expect("the result arrived");
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(journaled_results(&journal, "now-1"), 1);
        actor.after_change().await;
        assert_eq!(result_deliveries.load(Ordering::SeqCst), 1);
        assert_eq!(journaled_results(&journal, "now-1"), 1);
    }

    // #1067 oracle (Codex review on #12557): a schedule fire refused
    // at the fuel rate backs off until the window rolls instead of
    // waking the loop now — no retry is attempted before the retry
    // deadline — and the same occurrence then fires exactly once.
    #[tokio::test]
    async fn a_fuel_refused_schedule_fire_backs_off_until_rollover() {
        let (mut actor, clock, _token, _journal, _, fire_attempts) =
            fuel_window_actor("fuel-schedule", EPOCH_MS + 1_500);
        actor
            .core
            .schedules
            .insert("tick".to_owned(), PendingSchedule::one_shot(EPOCH_MS));
        // Before any refusal the overdue schedule wakes the loop now.
        assert_eq!(actor.next_deadline(), Some(clock.now_instant()));
        actor.fire_schedules().await;
        assert!(actor.program_healthy);
        assert!(actor.core.schedules.contains_key("tick"));
        assert_eq!(fire_attempts.load(Ordering::SeqCst), 1);
        // The refusal backs the next attempt off a full window.
        assert_eq!(
            actor.next_deadline(),
            Some(clock.now_instant() + Duration::from_secs(1))
        );
        // A second fire without advancing the clock attempts nothing.
        actor.fire_schedules().await;
        assert_eq!(fire_attempts.load(Ordering::SeqCst), 1);
        assert!(actor.core.schedules.contains_key("tick"));
        // At the retry deadline the fire is attempted again — refused
        // again while the window is still closed — and backs off
        // another full window.
        clock.advance(1_000);
        actor.fire_schedules().await;
        assert_eq!(fire_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            actor.next_deadline(),
            Some(clock.now_instant() + Duration::from_secs(1))
        );
        // Once the window rolls, the same occurrence fires exactly once.
        clock.advance(1_000);
        actor.fire_schedules().await;
        assert_eq!(fire_attempts.load(Ordering::SeqCst), 3);
        assert!(!actor.core.schedules.contains_key("tick"));
        assert_eq!(schedule_fires(&actor).len(), 1);
    }

    // #8239 oracle: staged spends count against the uses limit before
    // commit - a batch of three from a participant with two uses left
    // admits exactly two, and the third is refused at prepare time with
    // the single path's uses error.
    #[tokio::test]
    async fn input_batch_accounts_staged_spends_against_uses() {
        let bounds = CapabilityBounds {
            expires_at: None,
            uses: Some(2),
        };
        let (mut actor, token, journal) = batch_actor("batch-uses", bounds);
        let results = batch_replies(&mut actor, &token, &[b"one", b"two", b"three"]).await;
        assert!(results[0].is_ok(), "{:?}", results[0]);
        assert!(results[1].is_ok(), "{:?}", results[1]);
        let refused = format!("{:#}", results[2].as_ref().unwrap_err());
        assert!(refused.contains("no uses left"), "{refused}");
        assert_eq!(journal.committed().len(), 2);
    }

    #[tokio::test]
    async fn input_session_attribution_is_ephemeral_and_distinguishes_connections() {
        let (mut actor, _, token, journal) = scheduling_actor("sessions", &[], false);
        const SESSION_A: u64 = 0x5eed_cafe_0000_0001;
        const SESSION_B: u64 = 0x5eed_cafe_0000_0002;
        let session_ids = [SessionId::new(SESSION_A), SessionId::new(SESSION_B)];
        let mut participant = None;

        for session_id in session_ids {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::Input {
                    token: token.clone(),
                    input: b"{}".to_vec(),
                    session_id: Some(session_id),
                    device_input: None,
                    signed_move: None,
                    reply,
                })
                .await;

            let acting = actor
                .acting
                .expect("input participant is set while processing");
            if let Some(first) = participant {
                assert_eq!(acting, first);
            } else {
                participant = Some(acting);
            }
            assert!(actor.acting_session == Some(session_id));
            response.await.unwrap().unwrap();
            actor.after_change().await;
            actor.acting = None;
            actor.acting_session = None;
        }

        let committed: Vec<_> = journal
            .entries()
            .into_iter()
            .filter(|entry| matches!(entry, JournalEntry::Committed { .. }))
            .collect();
        assert_eq!(committed.len(), 2);
        for entry in committed {
            let value = serde_json::to_value(entry).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 2);
            assert!(value.get("session_id").is_none());
            let serialized = value.to_string();
            assert!(!serialized.contains(&SESSION_A.to_string()));
            assert!(!serialized.contains(&SESSION_B.to_string()));
        }
    }

    /// The records the program last published.
    fn records(actor: &WorldActor) -> Arc<Records> {
        actor.records_sender.borrow().1.clone()
    }

    /// The schedule fire events the program has received so far.
    fn schedule_fires(actor: &WorldActor) -> Vec<serde_json::Value> {
        records(actor)
            .get("fired")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// The result event the program received for request `id`, if any.
    fn schedule_result(actor: &WorldActor, id: &str) -> Option<serde_json::Value> {
        records(actor)
            .get("results")
            .and_then(|results| results.get(id))
            .cloned()
    }

    #[test]
    fn schedule_bounds_refuse_each_violation_with_a_reason() {
        let now = 1_000_000_000_000u64;
        let horizon = crate::world_caps::MAX_SCHEDULE_HORIZON_MS;
        let mut core = world(8);
        assert!(core.check_schedule("ok", now, now).is_ok());
        assert!(core.check_schedule(&"x".repeat(64), now, now).is_ok());
        let long = "x".repeat(65);
        for bad in ["", "tab\there", long.as_str()] {
            let error = core.check_schedule(bad, now, now).unwrap_err();
            assert!(error.to_string().contains("schedule id"), "{error}");
        }

        core.schedules
            .insert("taken".to_owned(), PendingSchedule::one_shot(now + 5));
        let error = core.check_schedule("taken", now + 9, now).unwrap_err();
        assert!(error.to_string().contains("already pending"), "{error}");
        assert_eq!(
            core.schedules["taken"].at_ms,
            now + 5,
            "a duplicate overwrites nothing"
        );

        assert!(core.check_schedule("edge", now + horizon, now).is_ok());
        let error = core
            .check_schedule("far", now + horizon + 1, now)
            .unwrap_err();
        assert!(error.to_string().contains("days past"), "{error}");
        assert!(core.check_schedule("past", 0, now).is_ok());

        let mut full = world(8);
        for index in 0..crate::world_caps::MAX_PENDING_SCHEDULES - 1 {
            full.schedules
                .insert(format!("s{index}"), PendingSchedule::one_shot(now));
        }
        assert!(full.check_schedule("last", now, now).is_ok());
        full.schedules
            .insert("last".to_owned(), PendingSchedule::one_shot(now));
        let error = full.check_schedule("one-more", now, now).unwrap_err();
        assert!(error.to_string().contains("pending schedules"), "{error}");
    }

    #[test]
    fn a_replacement_is_validated_as_a_replacement_not_as_an_addition() {
        // #807: the admin set that replaces a pending schedule validates
        // against the map as it will be after the replacement - the entry
        // being replaced is neither a duplicate nor a use of the capacity -
        // so validation can run before the cancellation is persisted, and a
        // refusal (here, past the horizon) leaves the original standing.
        let now = 1_000_000_000_000u64;
        let horizon = crate::world_caps::MAX_SCHEDULE_HORIZON_MS;
        let mut core = world(8);
        core.schedules
            .insert("taken".to_owned(), PendingSchedule::one_shot(now + 5));
        assert!(
            core.check_schedule_replacing("taken", now + 9, now).is_ok(),
            "replacing a pending id is not a duplicate"
        );
        let error = core
            .check_schedule_replacing("taken", now + horizon + 1, now)
            .unwrap_err();
        assert!(error.to_string().contains("days past"), "{error}");
        assert_eq!(
            core.schedules["taken"].at_ms,
            now + 5,
            "the refused replacement overwrites nothing"
        );

        let mut full = world(8);
        for index in 0..crate::world_caps::MAX_PENDING_SCHEDULES {
            full.schedules
                .insert(format!("s{index}"), PendingSchedule::one_shot(now));
        }
        assert!(
            full.check_schedule_replacing("s0", now + 9, now).is_ok(),
            "a replacement at capacity fits: it frees its own slot"
        );
        let error = full.check_schedule("one-more", now, now).unwrap_err();
        assert!(error.to_string().contains("pending schedules"), "{error}");
    }

    #[test]
    fn replacement_path_cannot_admit_a_new_id_past_the_schedule_cap() {
        let now = 1_000_000_000_000u64;
        let mut full = world(8);
        for index in 0..crate::world_caps::MAX_PENDING_SCHEDULES {
            full.schedules
                .insert(format!("s{index}"), PendingSchedule::one_shot(now));
        }

        let error = full
            .check_schedule_replacing("new", now + 9, now)
            .unwrap_err();
        assert!(error.to_string().contains("pending schedules"), "{error}");
        assert!(!full.schedules.contains_key("new"));
    }

    #[test]
    fn replacement_path_on_an_empty_schedule_map_does_not_underflow() {
        let now = 1_000_000_000_000u64;
        let core = world(8);

        assert!(core.check_schedule_replacing("new", now + 9, now).is_ok());
    }

    #[tokio::test]
    async fn an_ungranted_schedule_is_refused_and_counted() {
        let (mut actor, _clock, token, journal) = scheduling_actor("sched-refused", &[], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "nap", "at": EPOCH_MS + 60_000}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "s1").unwrap()["error"],
            "cap_denied"
        );
        assert!(
            actor.core.ledger.usage()["schedule.at"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::ScheduleSet { .. })),
            "a refused request pends nothing"
        );
    }

    /// Item 10-2 (#796/#5239): a schedule test that sets a ceiling.
    /// `schedule.at` is granted, but the world's capability ceiling excludes
    /// it, so the use is refused (and counted) and nothing pends — the
    /// grant itself is kept, so lifting the ceiling would restore it.
    #[tokio::test]
    async fn a_schedule_outside_the_ceiling_is_refused_and_pends_nothing() {
        let (mut actor, _clock, token, journal) =
            scheduling_actor("sched-ceiling", &["schedule.at"], false);
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling {
                ceiling: vec!["time.now".to_owned()],
                reply,
            })
            .await;
        let _ = response.await.unwrap().unwrap();
        actor.after_change().await;
        assert!(
            actor.core.ledger.granted().contains("schedule.at"),
            "the grant is kept under the ceiling"
        );
        assert!(
            !actor.core.ledger.is_granted("schedule.at"),
            "is_granted is allowed-and-granted: false while the ceiling excludes it"
        );
        assert!(
            !actor.core.ledger.effective().contains("schedule.at"),
            "the ceiling excludes schedule.at from the effective set"
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "nap", "at": EPOCH_MS + 60_000}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "s1").unwrap()["error"],
            "outside_ceiling"
        );
        assert!(
            actor.core.ledger.usage()["schedule.at"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            actor.core.schedules.is_empty(),
            "a ceiling-refused schedule pends nothing"
        );
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::ScheduleSet { .. })),
            "a ceiling-refused request journals no schedule set"
        );
    }

    // ---- F12.5 kv provider (#1062) ----

    #[test]
    fn kv_bounds_refuse_each_violation_with_a_reason() {
        let core = world(8);
        assert!(core.check_kv_set("ok", 4).is_ok());
        assert!(
            core.check_kv_set(&"x".repeat(crate::world_caps::MAX_KV_KEY_BYTES), 4)
                .is_ok()
        );
        let long = "x".repeat(crate::world_caps::MAX_KV_KEY_BYTES + 1);
        for bad in ["", "tab\there", long.as_str()] {
            let error = core.check_kv_set(bad, 4).unwrap_err();
            assert!(error.to_string().contains("kv key"), "{error}");
        }
        assert!(
            core.check_kv_set("ok", crate::world_caps::MAX_KV_VALUE_BYTES)
                .is_ok()
        );
        let error = core
            .check_kv_set("ok", crate::world_caps::MAX_KV_VALUE_BYTES + 1)
            .unwrap_err();
        assert!(error.to_string().contains("kv value"), "{error}");

        // Entry cap: a new key is refused at the cap; replacing an
        // existing key is not — it frees its own slot.
        let mut full = world(8);
        for index in 0..crate::world_caps::MAX_KV_ENTRIES {
            full.kv.insert(format!("k{index}"), "1".to_owned());
        }
        assert!(
            full.check_kv_set("k0", 4).is_ok(),
            "a replacement at capacity fits: it frees its own slot"
        );
        let error = full.check_kv_set("one-more", 4).unwrap_err();
        assert!(error.to_string().contains("kv entries"), "{error}");

        // Total byte cap, charged as key + value bytes and net of the
        // entry a replacement displaces.
        let mut store = world(8);
        store.kv.insert("a".to_owned(), "v".repeat(40_000));
        store.kv.insert("b".to_owned(), "v".repeat(20_000));
        let error = store.check_kv_set("c", 6_000).unwrap_err();
        assert!(error.to_string().contains("kv store"), "{error}");
        assert!(
            store.check_kv_set("a", 8).is_ok(),
            "shrinking an entry makes room under the total cap"
        );
    }

    #[tokio::test]
    async fn kv_set_get_delete_round_trip_through_the_actor() {
        let (mut actor, _clock, token, journal) =
            scheduling_actor("kv-round-trip", &["kv.get", "kv.set", "kv.delete"], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "w1", "cap": "kv.set", "args": {"key": "score", "value": {"best": 7}}},
            ]}),
        )
        .await;
        let written = schedule_result(&actor, "w1").unwrap();
        assert_eq!(written["ok"], true, "{written}");
        assert_eq!(actor.core.kv["score"], "{\"best\":7}");
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "r1", "cap": "kv.get", "args": {"key": "score"}},
                {"id": "r2", "cap": "kv.get", "args": {"key": "missing"}},
                {"id": "d1", "cap": "kv.delete", "args": {"key": "score"}},
            ]}),
        )
        .await;
        let read = schedule_result(&actor, "r1").unwrap();
        assert_eq!(
            read["value"]["value"],
            serde_json::json!({"best": 7}),
            "{read}"
        );
        let missing = schedule_result(&actor, "r2").unwrap();
        assert_eq!(
            missing["value"]["value"],
            serde_json::Value::Null,
            "{missing}"
        );
        let deleted = schedule_result(&actor, "d1").unwrap();
        assert_eq!(deleted["value"]["deleted"], true, "{deleted}");
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "r3", "cap": "kv.get", "args": {"key": "score"}},
                {"id": "d2", "cap": "kv.delete", "args": {"key": "score"}},
            ]}),
        )
        .await;
        let gone = schedule_result(&actor, "r3").unwrap();
        assert_eq!(gone["value"]["value"], serde_json::Value::Null, "{gone}");
        let deleted_again = schedule_result(&actor, "d2").unwrap();
        assert_eq!(deleted_again["value"]["deleted"], false, "{deleted_again}");
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::KvSet { .. }))
                .count(),
            1,
            "one set journals one entry"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::KvDeleted { .. }))
                .count(),
            1,
            "deleting an absent key journals nothing"
        );
    }

    #[tokio::test]
    async fn an_ungranted_kv_write_is_refused_and_counted() {
        let (mut actor, _clock, token, journal) = scheduling_actor("kv-refused", &[], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "w1", "cap": "kv.set", "args": {"key": "score", "value": 7}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "w1").unwrap()["error"],
            "cap_denied"
        );
        assert!(
            actor.core.ledger.usage()["kv.set"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(actor.core.kv.is_empty(), "a refused write stores nothing");
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::KvSet { .. })),
            "a refused write journals nothing"
        );
    }

    #[tokio::test]
    async fn a_kv_write_outside_the_ceiling_is_refused_and_stores_nothing() {
        let (mut actor, _clock, token, journal) =
            scheduling_actor("kv-ceiling", &["kv.set"], false);
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling {
                ceiling: vec!["time.now".to_owned()],
                reply,
            })
            .await;
        let _ = response.await.unwrap().unwrap();
        actor.after_change().await;
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "w1", "cap": "kv.set", "args": {"key": "score", "value": 7}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "w1").unwrap()["error"],
            "outside_ceiling"
        );
        assert!(
            actor.core.ledger.usage()["kv.set"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            actor.core.kv.is_empty(),
            "a ceiling-refused write stores nothing"
        );
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::KvSet { .. })),
            "a ceiling-refused write journals nothing"
        );
    }

    #[test]
    fn kv_entries_replay_and_survive_compaction() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_KV,
        };
        let set = |key: &str, value: &str| JournalEntry::KvSet {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        assert_eq!(
            journal_version_required_by(&set("a", "1")),
            JOURNAL_VERSION_KV
        );
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                header(),
                set("a", "1"),
                set("b", "{\"x\":2}"),
                set("a", "3"),
                JournalEntry::KvDeleted {
                    key: "b".to_owned(),
                },
            ],
        )
        .unwrap();
        assert_eq!(restored.core.kv.len(), 1);
        assert_eq!(restored.core.kv["a"], "3", "the last write to a key stands");
        // A delete of a key that is not stored refuses the restore.
        let bad = [
            header(),
            JournalEntry::KvDeleted {
                key: "ghost".to_owned(),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), bad).is_err());
        // Compaction: a core carrying entries re-emits them before its
        // checkpoint, so a restored core sees the same store.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.kv.insert("a".to_owned(), "3".to_owned());
        core.kv.insert("c".to_owned(), "[1,2]".to_owned());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(
            entries.contains(&set("a", "3")),
            "a compacted journal keeps the kv entry"
        );
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.kv, core.kv);
    }

    #[test]
    fn a_checkpoint_that_disagrees_with_its_kv_entries_is_refused() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_KV,
        };
        let set = || JournalEntry::KvSet {
            key: "a".to_owned(),
            value: "1".to_owned(),
        };
        let checkpoint = |kv: Option<Vec<CheckpointKvEntry>>| JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: Vec::new(),
            admin_history: None,
            open_presence: None,
        };
        let declared = || CheckpointKvEntry {
            key: "a".to_owned(),
            value: "1".to_owned(),
        };
        let agreed = [header(), set(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), agreed).is_ok());
        let omitted = [header(), set(), checkpoint(None)];
        assert!(WorldCore::restore("w", WorldLimits::default(), omitted).is_err());
        let unwritten = [header(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), unwritten).is_err());
        let duplicated = [
            header(),
            set(),
            checkpoint(Some(vec![declared(), declared()])),
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), duplicated).is_err());
    }

    #[test]
    fn a_journal_past_the_kv_bounds_is_refused() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_KV,
        };
        // A key the writer would never accept.
        let bad_key = [
            header(),
            JournalEntry::KvSet {
                key: String::new(),
                value: "1".to_owned(),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), bad_key).is_err());
        // A journal value must contain the JSON encoding promised by KvSet.
        let malformed_json = [
            header(),
            JournalEntry::KvSet {
                key: "a".to_owned(),
                value: "{\"x\":".to_owned(),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), malformed_json).is_err());
        // A value past the serialized cap.
        let big_value = [
            header(),
            JournalEntry::KvSet {
                key: "a".to_owned(),
                value: "v".repeat(crate::world_caps::MAX_KV_VALUE_BYTES + 1),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), big_value).is_err());
        // More entries than the store may hold.
        let mut entries = vec![header()];
        for index in 0..=crate::world_caps::MAX_KV_ENTRIES {
            entries.push(JournalEntry::KvSet {
                key: format!("k{index}"),
                value: "1".to_owned(),
            });
        }
        assert!(WorldCore::restore("w", WorldLimits::default(), entries).is_err());
    }

    // ---- F12.5 blob provider (#1062) ----

    #[test]
    fn blob_bounds_refuse_each_violation_with_a_reason() {
        let core = world(8);
        let hash = blob_id(b"content");
        assert!(core.check_blob_put(&hash, 7).is_ok());
        assert!(
            core.check_blob_put(&blob_id(b"max"), crate::world_caps::MAX_BLOB_BYTES)
                .is_ok()
        );
        let error = core.check_blob_put("not-a-hash", 4).unwrap_err();
        assert!(error.to_string().contains("blob hash"), "{error}");
        let error = core
            .check_blob_put(&hash, crate::world_caps::MAX_BLOB_BYTES + 1)
            .unwrap_err();
        assert!(error.to_string().contains("blob exceeds"), "{error}");

        // Entry cap: a new blob is refused at the cap; re-putting
        // content the world already holds is not — it is a no-op.
        let mut full = world(8);
        for index in 0..crate::world_caps::MAX_BLOBS {
            let content = vec![index as u8];
            full.blobs.insert(blob_id(&content), content);
        }
        assert!(
            full.check_blob_put(&blob_id(&[0u8]), 1).is_ok(),
            "a re-put at capacity fits: it frees no slot because it takes none"
        );
        let error = full.check_blob_put(&blob_id(b"one-more"), 8).unwrap_err();
        assert!(error.to_string().contains("blobs"), "{error}");

        // Total byte cap: 64 maximal blobs exactly fill it; one more
        // byte under a new address is refused, a re-put is not.
        let mut store = world(8);
        for index in 0..64u8 {
            let content = vec![index; crate::world_caps::MAX_BLOB_BYTES];
            store.blobs.insert(blob_id(&content), content);
        }
        assert_eq!(store.blob_bytes(), crate::world_caps::MAX_BLOB_TOTAL_BYTES);
        assert!(
            store
                .check_blob_put(&blob_id(&vec![0u8; crate::world_caps::MAX_BLOB_BYTES]), 4)
                .is_ok(),
            "a re-put at the total cap fits: it charges nothing new"
        );
        let error = store.check_blob_put(&blob_id(b"overflow"), 8).unwrap_err();
        assert!(error.to_string().contains("total bytes"), "{error}");
    }

    #[tokio::test]
    async fn blob_put_get_delete_round_trip_through_the_actor() {
        let (mut actor, _clock, token, journal) = scheduling_actor(
            "blob-round-trip",
            &["blob.put", "blob.get", "blob.delete"],
            false,
        );
        let data = blob_encode(b"hello blob");
        let hash = blob_id(b"hello blob");
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "p1", "cap": "blob.put", "args": {"data": data}},
            ]}),
        )
        .await;
        let written = schedule_result(&actor, "p1").unwrap();
        assert_eq!(written["ok"], true, "{written}");
        assert_eq!(written["value"]["hash"], hash, "{written}");
        assert_eq!(written["value"]["size"], 10, "{written}");
        assert_eq!(actor.core.blobs[&hash], b"hello blob");
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "p2", "cap": "blob.put", "args": {"data": data}},
                {"id": "g1", "cap": "blob.get", "args": {"hash": hash}},
                {"id": "g2", "cap": "blob.get", "args": {"hash": blob_id(b"never stored")}},
                {"id": "g3", "cap": "blob.get", "args": {"hash": "not-a-hash"}},
                {"id": "p3", "cap": "blob.put", "args": {"data": "!!! not base64 !!!"}},
            ]}),
        )
        .await;
        let again = schedule_result(&actor, "p2").unwrap();
        assert_eq!(again["ok"], true, "{again}");
        assert_eq!(again["value"]["hash"], hash, "{again}");
        let read = schedule_result(&actor, "g1").unwrap();
        assert_eq!(read["value"]["data"], data, "{read}");
        let missing = schedule_result(&actor, "g2").unwrap();
        assert_eq!(
            missing["value"]["data"],
            serde_json::Value::Null,
            "{missing}"
        );
        assert_eq!(
            schedule_result(&actor, "g3").unwrap()["error"],
            "bad_args",
            "a malformed hash is refused"
        );
        assert_eq!(
            schedule_result(&actor, "p3").unwrap()["error"],
            "bad_args",
            "malformed base64 is refused"
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "d1", "cap": "blob.delete", "args": {"hash": hash}},
            ]}),
        )
        .await;
        let deleted = schedule_result(&actor, "d1").unwrap();
        assert_eq!(deleted["value"]["deleted"], true, "{deleted}");
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "g4", "cap": "blob.get", "args": {"hash": hash}},
                {"id": "d2", "cap": "blob.delete", "args": {"hash": hash}},
            ]}),
        )
        .await;
        let gone = schedule_result(&actor, "g4").unwrap();
        assert_eq!(gone["value"]["data"], serde_json::Value::Null, "{gone}");
        let deleted_again = schedule_result(&actor, "d2").unwrap();
        assert_eq!(deleted_again["value"]["deleted"], false, "{deleted_again}");
        let entries = journal.entries();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::BlobStored { .. }))
                .count(),
            1,
            "two puts of one content journal one entry"
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::BlobRemoved { .. }))
                .count(),
            1,
            "deleting an absent blob journals nothing"
        );
    }

    struct DenyBlobGate;

    impl id_directory::directory::BoundaryGate for DenyBlobGate {
        fn check(
            &self,
            point: id_core::boundary::EnforcementPoint,
            _: &str,
        ) -> id_core::boundary_surfaces::FrameworkOutcome {
            if point == id_core::boundary::EnforcementPoint::BlobStorage {
                id_core::boundary_surfaces::FrameworkOutcome::Deny {
                    placement: Vec::new(),
                    assurance: Vec::new(),
                }
            } else {
                id_core::boundary_surfaces::FrameworkOutcome::Allow
            }
        }
    }

    #[tokio::test]
    async fn blob_boundary_denial_blocks_put_get_and_delete_before_effects() {
        // Oracle: a denied BlobStorage outcome refuses all three actor
        // operations and leaves both the in-memory blob set and journal intact.
        let (mut actor, _clock, token, journal) = scheduling_actor(
            "blob-boundary-denied",
            &["blob.put", "blob.get", "blob.delete"],
            false,
        );
        let bytes = b"protected blob".to_vec();
        let hash = blob_id(&bytes);
        actor.core.blobs.insert(hash.clone(), bytes.clone());
        actor.core.set_world_boundary_gate(Arc::new(DenyBlobGate));

        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "put", "cap": "blob.put", "args": {"data": blob_encode(b"new blob")}},
                {"id": "get", "cap": "blob.get", "args": {"hash": hash}},
                {"id": "delete", "cap": "blob.delete", "args": {"hash": hash}},
            ]}),
        )
        .await;

        for id in ["put", "get", "delete"] {
            assert_eq!(schedule_result(&actor, id).unwrap()["error"], "bad_args");
        }
        assert_eq!(actor.core.blobs.get(&hash), Some(&bytes));
        assert_eq!(
            journal
                .entries()
                .iter()
                .filter(|entry| matches!(
                    entry,
                    JournalEntry::BlobStored { .. } | JournalEntry::BlobRemoved { .. }
                ))
                .count(),
            0,
            "refused blob operations have no journal effects"
        );
    }

    #[tokio::test]
    async fn an_ungranted_blob_write_is_refused_and_counted() {
        let (mut actor, _clock, token, journal) = scheduling_actor("blob-refused", &[], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "p1", "cap": "blob.put", "args": {"data": blob_encode(b"content")}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "p1").unwrap()["error"],
            "cap_denied"
        );
        assert!(
            actor.core.ledger.usage()["blob.put"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            actor.core.blobs.is_empty(),
            "a refused write stores nothing"
        );
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::BlobStored { .. })),
            "a refused write journals nothing"
        );
    }

    #[tokio::test]
    async fn a_blob_write_outside_the_ceiling_is_refused_and_stores_nothing() {
        let (mut actor, _clock, token, journal) =
            scheduling_actor("blob-ceiling", &["blob.put"], false);
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetCapsCeiling {
                ceiling: vec!["time.now".to_owned()],
                reply,
            })
            .await;
        let _ = response.await.unwrap().unwrap();
        actor.after_change().await;
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "p1", "cap": "blob.put", "args": {"data": blob_encode(b"content")}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "p1").unwrap()["error"],
            "outside_ceiling"
        );
        assert!(
            actor.core.ledger.usage()["blob.put"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            actor.core.blobs.is_empty(),
            "a ceiling-refused write stores nothing"
        );
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::BlobStored { .. })),
            "a ceiling-refused write journals nothing"
        );
    }

    #[test]
    fn blob_entries_replay_and_survive_compaction() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_BLOB,
        };
        let stored = |content: &[u8]| JournalEntry::BlobStored {
            hash: blob_id(content),
            data: blob_encode(content),
        };
        assert_eq!(
            journal_version_required_by(&stored(b"a")),
            JOURNAL_VERSION_BLOB
        );
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [
                header(),
                stored(b"alpha"),
                stored(b"beta"),
                JournalEntry::BlobRemoved {
                    hash: blob_id(b"beta"),
                },
            ],
        )
        .unwrap();
        assert_eq!(restored.core.blobs.len(), 1);
        assert_eq!(restored.core.blobs[&blob_id(b"alpha")], b"alpha");
        // A remove of a blob that is not stored refuses the restore.
        let bad = [
            header(),
            JournalEntry::BlobRemoved {
                hash: blob_id(b"ghost"),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), bad).is_err());
        // Content that does not hash to its stated address refuses
        // the restore: the journal is self-verifying.
        let tampered = [
            header(),
            JournalEntry::BlobStored {
                hash: blob_id(b"alpha"),
                data: blob_encode(b"forged"),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), tampered).is_err());
        // Content that is not base64 at all refuses it too.
        let garbled = [
            header(),
            JournalEntry::BlobStored {
                hash: blob_id(b"alpha"),
                data: "!!! not base64 !!!".to_owned(),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), garbled).is_err());
        // Compaction: a core carrying blobs re-emits them before its
        // checkpoint, so a restored core sees the same set.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.blobs.insert(blob_id(b"alpha"), b"alpha".to_vec());
        core.blobs.insert(blob_id(b"gamma"), b"gamma".to_vec());
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(
            entries.contains(&stored(b"alpha")),
            "a compacted journal keeps the blob entry"
        );
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.blobs, core.blobs);
    }

    #[test]
    fn a_checkpoint_that_disagrees_with_its_blob_index_is_refused() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_BLOB,
        };
        let stored = || JournalEntry::BlobStored {
            hash: blob_id(b"alpha"),
            data: blob_encode(b"alpha"),
        };
        let checkpoint = |blobs: Option<Vec<CheckpointBlobEntry>>| JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: Vec::new(),
            admin_history: None,
            open_presence: None,
        };
        let declared = || CheckpointBlobEntry {
            hash: blob_id(b"alpha"),
            size: 5,
        };
        let agreed = [header(), stored(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), agreed).is_ok());
        let omitted = [header(), stored(), checkpoint(None)];
        assert!(WorldCore::restore("w", WorldLimits::default(), omitted).is_err());
        let unwritten = [header(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), unwritten).is_err());
        let wrong_size = [
            header(),
            stored(),
            checkpoint(Some(vec![CheckpointBlobEntry {
                hash: blob_id(b"alpha"),
                size: 4,
            }])),
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), wrong_size).is_err());
        let duplicated = [
            header(),
            stored(),
            checkpoint(Some(vec![declared(), declared()])),
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), duplicated).is_err());
    }

    #[test]
    fn a_journal_past_the_blob_bounds_is_refused() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_BLOB,
        };
        // A hash the writer would never accept.
        let bad_hash = [
            header(),
            JournalEntry::BlobStored {
                hash: "not-a-hash".to_owned(),
                data: blob_encode(b"a"),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), bad_hash).is_err());
        // A blob past the content cap, under its true address.
        let big = vec![7u8; crate::world_caps::MAX_BLOB_BYTES + 1];
        let big_blob = [
            header(),
            JournalEntry::BlobStored {
                hash: blob_id(&big),
                data: blob_encode(&big),
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), big_blob).is_err());
        // More blobs than the set may hold.
        let mut entries = vec![header()];
        for index in 0..=crate::world_caps::MAX_BLOBS {
            let content = (index as u32).to_le_bytes();
            entries.push(JournalEntry::BlobStored {
                hash: blob_id(&content),
                data: blob_encode(&content),
            });
        }
        assert!(WorldCore::restore("w", WorldLimits::default(), entries).is_err());
        // More content in total than the set may hold.
        let mut entries = vec![header()];
        for index in 0..65u8 {
            let content = vec![index; crate::world_caps::MAX_BLOB_BYTES];
            entries.push(JournalEntry::BlobStored {
                hash: blob_id(&content),
                data: blob_encode(&content),
            });
        }
        assert!(WorldCore::restore("w", WorldLimits::default(), entries).is_err());
    }

    // ---- F12.5 http.fetch provider (#1062) ----

    /// A hermetic HTTP/1.1 server on a loopback port: one canned
    /// response per path, `Connection: close` on every reply, and
    /// a hit count so refusal tests can prove no request arrived.
    /// No test in this section touches the live internet.
    struct FetchServer {
        port: u16,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl FetchServer {
        fn hits(&self) -> usize {
            self.hits.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }

        /// The allowlist entry for this server: its host and port.
        /// A bare `127.0.0.1` would admit only port 80.
        fn entry(&self) -> String {
            format!("127.0.0.1:{}", self.port)
        }
    }

    fn fetch_server() -> FetchServer {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            request.extend_from_slice(&buffer[..read]);
                            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, headers, body): (&str, String, Vec<u8>) = match path.as_str() {
                    "/hello" => ("200 OK", String::new(), b"hello fetch".to_vec()),
                    "/hop" => ("302 Found", "Location: /hello\r\n".to_owned(), Vec::new()),
                    "/away" => (
                        "302 Found",
                        format!("Location: http://localhost:{port}/hello\r\n"),
                        Vec::new(),
                    ),
                    "/default-port" => (
                        "302 Found",
                        "Location: http://127.0.0.1/hello\r\n".to_owned(),
                        Vec::new(),
                    ),
                    "/loop" => ("302 Found", "Location: /loop\r\n".to_owned(), Vec::new()),
                    "/big" => (
                        "200 OK",
                        String::new(),
                        vec![b'x'; crate::world_caps::MAX_HTTP_FETCH_RESPONSE_BYTES + 1],
                    ),
                    _ => ("404 Not Found", String::new(), b"nope".to_vec()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        FetchServer { port, hits }
    }

    /// An actor for `http.fetch` tests, on `scheduling_actor`'s
    /// pattern. `enabled` performs the operator's two ceiling
    /// acts — naming the provider in the server caps and in the
    /// world's own ceiling; without them the provider stays off
    /// by default however it is granted.
    fn http_fetch_actor(
        id: &str,
        grants: &[&str],
        hosts: &[&str],
        enabled: bool,
    ) -> (WorldActor, Arc<ManualClock>, JoinCapability, Recorded) {
        let journal = Recorded::for_world(id);
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut limits = WorldLimits::default();
        if enabled {
            limits.server_caps.insert("http.fetch".to_owned());
        }
        limits.http_fetch_hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
        let mut core = WorldCore::new_with_clock(id, limits, clock.clone()).unwrap();
        if enabled {
            core.caps_ceiling.insert("http.fetch".to_owned());
            core.sync_ledger_ceiling();
        }
        core.ledger
            .set_granted(grants.iter().map(|grant| (*grant).to_owned()).collect());
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let issued = core.issued_entry(ann.id).unwrap();
        super::tests::push_bumped(&mut journal.0.lock().unwrap(), &issued);
        let program = Scheduler::default();
        let actor = WorldActor::for_test(core, Box::new(program), Some(Box::new(journal.clone())));
        (actor, clock, token, journal)
    }

    #[test]
    fn http_fetch_checks_refuse_each_violation_with_a_reason() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut limits = WorldLimits::default();
        // An unnormalized entry still matches: entries and URL
        // hosts pass through the same normalization.
        limits.http_fetch_hosts = ["EXAMPLE.com."].into_iter().map(str::to_owned).collect();
        let mut core = WorldCore::new_with_clock("w", limits, clock.clone()).unwrap();

        let host = core.check_http_fetch("https://example.com/a?b=1").unwrap();
        assert_eq!(host, "example.com");
        let error = core.check_http_fetch("not a url").unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
        let error = core.check_http_fetch("ftp://example.com/").unwrap_err();
        assert!(error.to_string().contains("scheme"), "{error}");
        let error = core
            .check_http_fetch("https://user@example.com/")
            .unwrap_err();
        assert!(error.to_string().contains("credentials"), "{error}");
        let error = core.check_http_fetch("https://other.example/").unwrap_err();
        assert!(
            error.to_string().contains("not in the allowlist"),
            "{error}"
        );
        // Subdomains are not implied by listing the parent.
        let error = core
            .check_http_fetch("https://api.example.com/")
            .unwrap_err();
        assert!(
            error.to_string().contains("not in the allowlist"),
            "{error}"
        );
        // A bare host admits only the scheme's default port.
        assert!(core.check_http_fetch("https://example.com:443/").is_ok());
        let error = core
            .check_http_fetch("https://example.com:8443/")
            .unwrap_err();
        assert!(
            error.to_string().contains("not in the allowlist"),
            "{error}"
        );
        // Listing `host:port` opens that port and no other.
        core.limits
            .http_fetch_hosts
            .insert("example.com:8443".to_owned());
        assert!(core.check_http_fetch("https://example.com:8443/").is_ok());
        let error = core
            .check_http_fetch("https://example.com:8444/")
            .unwrap_err();
        assert!(
            error.to_string().contains("not in the allowlist"),
            "{error}"
        );

        // The rate bound: a full window of attempts refuses the
        // next check, and the window admits again once it passes.
        for _ in 0..crate::world_caps::MAX_HTTP_FETCH_BURST {
            core.check_http_fetch("https://example.com/").unwrap();
            core.note_http_fetch_attempt();
        }
        let error = core.check_http_fetch("https://example.com/").unwrap_err();
        assert!(error.to_string().contains("rate"), "{error}");
        clock.advance(crate::world_caps::HTTP_FETCH_RATE_WINDOW_MS + 1);
        assert!(core.check_http_fetch("https://example.com/").is_ok());
    }

    #[tokio::test]
    async fn http_fetch_is_off_by_default() {
        let server = fetch_server();
        // Granted, and the host allowlisted — but the operator
        // never named the provider in a ceiling, so the default
        // ceiling refuses it before any provider code runs.
        let (mut actor, _clock, token, _journal) = http_fetch_actor(
            "fetch-default-off",
            &["http.fetch"],
            &[server.entry().as_str()],
            false,
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "f1").unwrap()["error"],
            "outside_ceiling"
        );
        assert_eq!(server.hits(), 0, "a refused fetch connects nowhere");
    }

    #[tokio::test]
    async fn an_ungranted_http_fetch_is_refused_and_counted() {
        let server = fetch_server();
        let (mut actor, _clock, token, _journal) =
            http_fetch_actor("fetch-ungranted", &[], &[server.entry().as_str()], true);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "f1").unwrap()["error"],
            "cap_denied"
        );
        assert!(
            actor.core.ledger.usage()["http.fetch"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert_eq!(server.hits(), 0, "a refused fetch connects nowhere");
    }

    #[tokio::test]
    async fn an_unlisted_host_is_refused() {
        let server = fetch_server();
        // Enabled and granted, but the allowlist names another
        // host: the fetch is refused before any connection.
        let (mut actor, _clock, token, _journal) =
            http_fetch_actor("fetch-unlisted", &["http.fetch"], &["example.org"], true);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
            ]}),
        )
        .await;
        let refused = schedule_result(&actor, "f1").unwrap();
        assert_eq!(refused["error"], "bad_args", "{refused}");
        assert!(
            refused["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("not in the allowlist"),
            "{refused}"
        );
        assert_eq!(server.hits(), 0, "an unlisted host is never contacted");
    }

    #[tokio::test]
    async fn a_bare_host_entry_refuses_a_non_default_port() {
        let server = fetch_server();
        // The host is listed without a port, so only port 80 is
        // open; the server's own port is refused before any
        // connection is made.
        let (mut actor, _clock, token, _journal) =
            http_fetch_actor("fetch-bare-host", &["http.fetch"], &["127.0.0.1"], true);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
            ]}),
        )
        .await;
        let refused = schedule_result(&actor, "f1").unwrap();
        assert_eq!(refused["error"], "bad_args", "{refused}");
        assert!(
            refused["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("not in the allowlist"),
            "{refused}"
        );
        assert_eq!(server.hits(), 0, "an unlisted port is never contacted");
    }

    #[tokio::test]
    async fn http_fetch_round_trip_through_the_actor() {
        let server = fetch_server();
        let (mut actor, _clock, token, _journal) = http_fetch_actor(
            "fetch-round-trip",
            &["http.fetch"],
            &[server.entry().as_str()],
            true,
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
                {"id": "f2", "cap": "http.fetch", "args": {"url": server.url("/hop")}},
                {"id": "f3", "cap": "http.fetch", "args": {"url": server.url("/missing")}},
            ]}),
        )
        .await;
        let fetched = schedule_result(&actor, "f1").unwrap();
        assert_eq!(fetched["ok"], true, "{fetched}");
        assert_eq!(fetched["value"]["status"], 200, "{fetched}");
        assert_eq!(fetched["value"]["url"], server.url("/hello"), "{fetched}");
        assert_eq!(
            fetched["value"]["body"],
            blob_encode(b"hello fetch"),
            "{fetched}"
        );
        assert_eq!(fetched["value"]["body_bytes"], 11, "{fetched}");
        // An on-list redirect is followed to its target.
        let hopped = schedule_result(&actor, "f2").unwrap();
        assert_eq!(hopped["ok"], true, "{hopped}");
        assert_eq!(hopped["value"]["status"], 200, "{hopped}");
        assert_eq!(hopped["value"]["url"], server.url("/hello"), "{hopped}");
        assert_eq!(hopped["value"]["body"], blob_encode(b"hello fetch"));
        // A status is a result, not a refusal: only policy and
        // transport failures refuse a fetch.
        let missing = schedule_result(&actor, "f3").unwrap();
        assert_eq!(missing["ok"], true, "{missing}");
        assert_eq!(missing["value"]["status"], 404, "{missing}");
    }

    #[tokio::test]
    async fn http_fetch_refuses_redirects_off_the_list_and_past_the_hop_cap() {
        let server = fetch_server();
        let (mut actor, _clock, token, _journal) = http_fetch_actor(
            "fetch-redirects",
            &["http.fetch"],
            &[server.entry().as_str()],
            true,
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/away")}},
                {"id": "f2", "cap": "http.fetch", "args": {"url": server.url("/loop")}},
                {"id": "f3", "cap": "http.fetch", "args": {"url": server.url("/big")}},
                {"id": "f4", "cap": "http.fetch", "args": {"url": server.url("/default-port")}},
            ]}),
        )
        .await;
        // /away redirects to localhost, which is not allowlisted:
        // refused before the second connection is made.
        let away = schedule_result(&actor, "f1").unwrap();
        assert_eq!(away["error"], "bad_args", "{away}");
        assert!(
            away["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("host_not_allowed"),
            "{away}"
        );
        let looping = schedule_result(&actor, "f2").unwrap();
        assert_eq!(looping["error"], "failed", "{looping}");
        assert!(
            looping["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("too_many_redirects"),
            "{looping}"
        );
        let big = schedule_result(&actor, "f3").unwrap();
        assert_eq!(big["error"], "failed", "{big}");
        assert!(
            big["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("response_too_large"),
            "{big}"
        );
        // /default-port redirects to the same host on port 80,
        // which the `host:port` entry does not list: refused.
        let other_port = schedule_result(&actor, "f4").unwrap();
        assert_eq!(other_port["error"], "bad_args", "{other_port}");
        assert!(
            other_port["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("host_not_allowed"),
            "{other_port}"
        );
        // Exactly the requests made: /away once (the redirect was
        // not followed), /loop for the initial fetch plus the hop
        // cap of follows, /big once, /default-port once.
        assert_eq!(
            server.hits(),
            1 + (1 + crate::world_caps::MAX_HTTP_FETCH_REDIRECTS) + 1 + 1
        );
    }

    #[tokio::test]
    async fn http_fetches_journal_nothing_and_replay_is_unaffected() {
        let server = fetch_server();
        let (mut actor, _clock, token, journal) = http_fetch_actor(
            "fetch-replay",
            &["kv.set", "http.fetch"],
            &[server.entry().as_str()],
            true,
        );
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "k1", "cap": "kv.set", "args": {"key": "a", "value": 1}},
            ]}),
        )
        .await;
        let before = journal.entries();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "f1", "cap": "http.fetch", "args": {"url": server.url("/hello")}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "f1").unwrap()["ok"],
            true,
            "the fetch itself succeeded"
        );
        // The fetch ask adds only the entries every request adds:
        // the input's committed event and the result's system
        // event (the delivery replay feeds the program, as for a
        // `kv.get`). No entry describes the fetch as state for
        // replay to apply — there is no fetch entry kind at all.
        let after = journal.entries();
        let added = &after[before.len()..];
        assert!(
            added.iter().all(|entry| matches!(
                entry,
                JournalEntry::Committed { .. } | JournalEntry::System { .. }
            )),
            "a fetch journals no state of its own: {added:?}"
        );
        // And the journal — written across the fetch — restores
        // exactly the state the journaled writes describe.
        let restored = WorldCore::restore("fetch-replay", WorldLimits::default(), after).unwrap();
        assert_eq!(restored.core.kv["a"], "1");
        assert!(restored.core.blobs.is_empty());
    }

    /// An actor for `notify` tests, on `http_fetch_actor`'s
    /// pattern. `enabled` performs the operator's two ceiling
    /// acts — naming the provider in the server caps and in the
    /// world's own ceiling; without them the provider stays off
    /// by default however it is granted.
    fn notify_actor(
        id: &str,
        grants: &[&str],
        enabled: bool,
    ) -> (WorldActor, Arc<ManualClock>, JoinCapability, Recorded) {
        let journal = Recorded::for_world(id);
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut limits = WorldLimits::default();
        if enabled {
            limits.server_caps.insert("notify".to_owned());
        }
        let mut core = WorldCore::new_with_clock(id, limits, clock.clone()).unwrap();
        if enabled {
            core.caps_ceiling.insert("notify".to_owned());
            core.sync_ledger_ceiling();
        }
        core.ledger
            .set_granted(grants.iter().map(|grant| (*grant).to_owned()).collect());
        let (ann, token) = core
            .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        let issued = core.issued_entry(ann.id).unwrap();
        super::tests::push_bumped(&mut journal.0.lock().unwrap(), &issued);
        let program = Scheduler::default();
        let actor = WorldActor::for_test(core, Box::new(program), Some(Box::new(journal.clone())));
        (actor, clock, token, journal)
    }

    #[test]
    fn notify_checks_refuse_each_violation_with_a_reason() {
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core =
            WorldCore::new_with_clock("w", WorldLimits::default(), clock.clone()).unwrap();

        core.check_notify("the match has ended").unwrap();
        // Oversized: past the world's chat byte limit — the bound
        // a notification shares with the host's own chat lines.
        let big = "x".repeat(core.limits.chat_bytes + 1);
        let error = core.check_notify(&big).unwrap_err();
        assert!(error.to_string().contains("exceeds"), "{error}");
        // Empty, and empty once the sanitizer strips it.
        let error = core.check_notify("").unwrap_err();
        assert!(error.to_string().contains("empty"), "{error}");
        let error = core.check_notify("\u{1}\u{2}").unwrap_err();
        assert!(error.to_string().contains("empty"), "{error}");

        // The rate bound: a full window of attempts refuses the
        // next check, and the window admits again once it passes.
        for _ in 0..crate::world_caps::MAX_NOTIFY_BURST {
            core.check_notify("again").unwrap();
            core.note_notify_attempt();
        }
        let error = core.check_notify("again").unwrap_err();
        assert!(error.to_string().contains("rate"), "{error}");
        clock.advance(crate::world_caps::NOTIFY_RATE_WINDOW_MS + 1);
        assert!(core.check_notify("again").is_ok());
    }

    #[tokio::test]
    async fn notify_is_off_by_default() {
        // Granted — but the operator never named the provider in a
        // ceiling, so the default ceiling refuses it before any
        // provider code runs, and nothing is committed.
        let (mut actor, _clock, token, _journal) =
            notify_actor("notify-default-off", &["notify"], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "n1", "cap": "notify", "args": {"text": "hello"}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "n1").unwrap()["error"],
            "outside_ceiling"
        );
        assert!(
            actor
                .core
                .events
                .iter()
                .all(|event| !matches!(event.kind, WorldEventKind::Notification { .. })),
            "a refused notify commits no event"
        );
    }

    #[tokio::test]
    async fn an_ungranted_notify_is_refused_and_counted() {
        let (mut actor, _clock, token, _journal) = notify_actor("notify-ungranted", &[], true);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "n1", "cap": "notify", "args": {"text": "hello"}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "n1").unwrap()["error"],
            "cap_denied"
        );
        assert!(
            actor.core.ledger.usage()["notify"].denied > 0,
            "{:?}",
            actor.core.ledger.usage()
        );
        assert!(
            actor
                .core
                .events
                .iter()
                .all(|event| !matches!(event.kind, WorldEventKind::Notification { .. })),
            "a refused notify commits no event"
        );
    }

    #[tokio::test]
    async fn notify_round_trip_through_the_actor() {
        let (mut actor, _clock, token, _journal) =
            notify_actor("notify-round-trip", &["notify"], true);
        let mut broadcast = actor.event_sender.subscribe();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "n1", "cap": "notify", "args": {"text": "the match has ended\u{7}"}},
            ]}),
        )
        .await;
        let sent = schedule_result(&actor, "n1").unwrap();
        assert_eq!(sent["ok"], true, "{sent}");
        let sequence = sent["value"]["sequence"].as_u64().unwrap();
        // The committed event is in the world's log, host-authored,
        // carrying the sanitized text (the bell is stripped).
        let event = actor
            .core
            .events
            .iter()
            .find(|event| event.sequence == sequence)
            .expect("the notification is in the event log");
        assert_eq!(event.participant_id, 0);
        match &event.kind {
            WorldEventKind::Notification { text } => {
                assert_eq!(text, "the match has ended");
            }
            other => panic!("expected a notification event, got {other:?}"),
        }
        // And it went out on the broadcast channel the sessions
        // read (the ask's own input event may precede it there).
        let mut aired = None;
        while let Ok(event) = broadcast.try_recv() {
            if matches!(event.kind, WorldEventKind::Notification { .. }) {
                aired = Some(event);
            }
        }
        let aired = aired.expect("the notification was broadcast");
        assert_eq!(aired.sequence, sequence);
    }

    #[tokio::test]
    async fn notify_refuses_past_the_rate_bound() {
        let (mut actor, _clock, token, _journal) = notify_actor("notify-rate", &["notify"], true);
        // One wants document, burst + 1 requests: the window
        // admits the burst and refuses the next in the same change.
        let requests: Vec<serde_json::Value> = (1..=crate::world_caps::MAX_NOTIFY_BURST + 1)
            .map(|index| {
                serde_json::json!({"id": format!("n{index}"), "cap": "notify", "args": {"text": "ping"}})
            })
            .collect();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": requests}),
        )
        .await;
        for index in 1..=crate::world_caps::MAX_NOTIFY_BURST {
            assert_eq!(
                schedule_result(&actor, &format!("n{index}")).unwrap()["ok"],
                true,
                "attempt {index} is inside the burst"
            );
        }
        let refused = schedule_result(
            &actor,
            &format!("n{}", crate::world_caps::MAX_NOTIFY_BURST + 1),
        )
        .unwrap();
        assert_eq!(refused["error"], "bad_args", "{refused}");
        assert!(
            refused["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("rate"),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn notify_journals_a_committed_event_and_replay_restores_it() {
        let (mut actor, _clock, token, journal) = notify_actor("notify-replay", &["notify"], true);
        let before = journal.entries();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "n1", "cap": "notify", "args": {"text": "server restarting soon"}},
            ]}),
        )
        .await;
        assert_eq!(schedule_result(&actor, "n1").unwrap()["ok"], true);
        // The notify ask adds the entries every request adds (the
        // input's committed event, the result's system event) plus
        // exactly one of its own: the committed notification.
        let after = journal.entries();
        let added = &after[before.len()..];
        let notifications: Vec<&JournalEntry> = added
            .iter()
            .filter(|entry| {
                matches!(
                    entry,
                    JournalEntry::Committed { event }
                        if matches!(event.kind, WorldEventKind::Notification { .. })
                )
            })
            .collect();
        assert_eq!(notifications.len(), 1, "{added:?}");
        assert!(
            added.iter().all(|entry| matches!(
                entry,
                JournalEntry::Committed { .. } | JournalEntry::System { .. }
            )),
            "a notify journals no state of its own beyond its event: {added:?}"
        );
        // The notification entry is a version-15 entry: a journal
        // carrying one is refused by older binaries at the header
        // check, never failed mid-replay.
        assert_eq!(
            journal_version_required_by(notifications[0]),
            JOURNAL_VERSION_NOTIFY
        );
        // Restoring from the journal rebuilds the event log with
        // the notification intact at its sequence.
        let restored = WorldCore::restore("notify-replay", WorldLimits::default(), after).unwrap();
        assert!(
            restored.core.events.iter().any(|event| matches!(
                &event.kind,
                WorldEventKind::Notification { text } if text == "server restarting soon"
            )),
            "{:?}",
            restored.core.events
        );
    }

    #[test]
    fn notify_events_require_journal_version_15() {
        let event = WorldEvent {
            sequence: 1,
            participant_id: 0,
            kind: WorldEventKind::Notification {
                text: "hi".to_owned(),
            },
        };
        assert_eq!(
            journal_version_required_by(&JournalEntry::Committed {
                event: event.clone()
            }),
            JOURNAL_VERSION_NOTIFY
        );
        // A checkpoint retaining one requires it too.
        let checkpoint = JournalEntry::Checkpoint {
            sequence: 1,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules: None,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: vec![event],
            open_presence: None,
            admin_history: None,
        };
        assert_eq!(
            journal_version_required_by(&checkpoint),
            JOURNAL_VERSION_NOTIFY
        );
        // Version 16 (#13711, stamped lease positions) is the newest
        // version this binary reads.
        assert_eq!(MAX_JOURNAL_VERSION, JOURNAL_VERSION_LEASE_POSITION);
    }

    #[tokio::test]
    async fn a_due_schedule_fires_once_and_the_program_receives_it() {
        let (mut actor, clock, token, journal) =
            scheduling_actor("sched-fire", &["schedule.at"], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "now", "at": EPOCH_MS + 1}},
            ]}),
        )
        .await;
        // Not due at the epoch; only after the clock passes the due time.
        actor.fire_schedules().await;
        assert!(schedule_fires(&actor).is_empty(), "not due yet");
        clock.advance(1);
        actor.fire_schedules().await;
        let fires = schedule_fires(&actor);
        assert_eq!(fires.len(), 1, "{fires:?}");
        assert_eq!(fires[0]["id"], "now");
        assert!(
            fires[0].get("late").is_none(),
            "a fire in this run is not late"
        );
        // A second wakeup fires nothing: the schedule was consumed.
        actor.fire_schedules().await;
        assert_eq!(schedule_fires(&actor).len(), 1);

        let entries = journal.entries();
        let fire = WorldEventKind::System {
            event: schedule_fired_event("now", false),
        };
        let delivered = entries
            .iter()
            .position(|entry| matches!(entry, JournalEntry::System { event } if event.kind == fire))
            .expect("the fire is journaled as delivered");
        let consumed = entries
            .iter()
            .position(|entry| matches!(entry, JournalEntry::ScheduleFired { id } if id == "now"))
            .expect("the schedule is consumed");
        assert!(delivered < consumed, "the delivery is journaled first");
        assert_eq!(
            entries
                .iter()
                .filter(|entry| matches!(entry, JournalEntry::ScheduleFired { .. }))
                .count(),
            1
        );
    }

    /// The ids of the journal's `ScheduleFired` entries, in journal order:
    /// the fire history an operator reads from the world's journal.
    fn journaled_fires(journal: &Recorded) -> Vec<String> {
        journal
            .entries()
            .into_iter()
            .filter_map(|entry| match entry {
                JournalEntry::ScheduleFired { id } => Some(id),
                _ => None,
            })
            .collect()
    }

    /// #8342 (#796 item 10-2, the firing ceiling): a firing storm cannot
    /// exceed [`crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP`] fires in
    /// one wakeup.
    ///
    /// Oracle: that declared constant, checked against the journal's
    /// `ScheduleFired` entries (the visible fire history). No live path can
    /// pend more schedules than the bound, because
    /// [`crate::world_caps::MAX_PENDING_SCHEDULES`] is no larger, so the
    /// storm is planted directly to exercise the truncation itself: the
    /// earliest due fire first, and the rest fire on the next wakeup with
    /// none lost.
    #[tokio::test]
    async fn a_firing_storm_is_bounded_per_wakeup_and_the_rest_fire_next() {
        let bound = crate::world_caps::MAX_SCHEDULE_FIRES_PER_WAKEUP;
        assert!(
            crate::world_caps::MAX_PENDING_SCHEDULES <= bound,
            "a world's pending cap fits in one wakeup"
        );
        let extra = 8;
        let (mut actor, clock, _token, journal) =
            scheduling_actor("sched-storm", &["schedule.at"], false);
        for index in 0..bound + extra {
            let at = EPOCH_MS + u64::try_from(index).unwrap();
            actor
                .core
                .schedules
                .insert(format!("s{index:03}"), PendingSchedule::one_shot(at));
        }
        clock.advance(u64::try_from(bound + extra).unwrap());

        actor.fire_schedules().await;
        let first = journaled_fires(&journal);
        assert_eq!(first.len(), bound, "one wakeup fires at most the bound");
        let earliest: Vec<String> = (0..bound).map(|index| format!("s{index:03}")).collect();
        assert_eq!(first, earliest, "the earliest due schedules fire first");
        assert_eq!(
            schedule_fires(&actor).len(),
            bound,
            "the program received exactly the journaled fires"
        );
        assert_eq!(actor.core.schedules.len(), extra, "the rest stay pending");

        actor.fire_schedules().await;
        assert_eq!(journaled_fires(&journal).len(), bound + extra);
        assert!(actor.core.schedules.is_empty(), "no fire was lost");
        actor.fire_schedules().await;
        assert_eq!(
            journaled_fires(&journal).len(),
            bound + extra,
            "nothing fires twice"
        );
    }

    /// #8342: a recurring schedule that fell many intervals behind (a host
    /// asleep, say) fires once per wakeup, never a catch-up burst, and its
    /// series still ends at its count. Oracle: one journaled `ScheduleFired`
    /// per wakeup and the schedule's `remaining` count.
    #[tokio::test]
    async fn a_recurring_schedule_far_behind_fires_once_per_wakeup() {
        let (mut actor, clock, _token, journal) =
            scheduling_actor("sched-behind", &["schedule.at"], false);
        let remaining = 3;
        actor.core.schedules.insert(
            "r".to_owned(),
            PendingSchedule {
                at_ms: EPOCH_MS + 10,
                interval_ms: Some(10),
                remaining,
                target: None,
            },
        );
        clock.advance(10_000);
        for wakeup in 1..=remaining {
            actor.fire_schedules().await;
            assert_eq!(
                journaled_fires(&journal).len(),
                usize::try_from(wakeup).unwrap(),
                "wakeup {wakeup} fires the overdue series once"
            );
        }
        assert!(actor.core.schedules.is_empty(), "the series ended");
        actor.fire_schedules().await;
        assert_eq!(
            journaled_fires(&journal),
            vec!["r".to_owned(); usize::try_from(remaining).unwrap()]
        );
    }

    /// Item 10-2 (#796/#5239): a schedule fire never reaches joiners.
    /// The fire is a host `System` event delivered to the program; a
    /// joiner's catch-up page (`events_after`) must not carry it, while a
    /// genuine participant event on the same page still comes through.
    #[test]
    fn a_schedule_fire_never_reaches_joiners_via_events_after() {
        let mut core = world(8);
        let (_, token) = core
            .issue_capability("Ada", WorldScopes::GUEST, CapabilityBounds::default())
            .unwrap();
        // Positive control: a participant event is visible to joiners.
        assert_eq!(core.input(&token, b"move").unwrap().sequence, 1);
        // A delivered fire, retained as host events are, carrying the
        // schedule-fired payload for schedule "siesta" (an id that is
        // not a substring of any EventPage field name — "nap" is one
        // of needs_snapshot, which false-positives a whole-page scan).
        let fire = WorldEvent {
            sequence: 2,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: schedule_fired_event("siesta", false),
            },
        };
        core.sequence = fire.sequence;
        core.events.push_back(fire);
        let page = core.events_after(&token, 0).unwrap();
        assert_eq!(
            page.events.len(),
            1,
            "only the participant event reaches a joiner: {page:?}"
        );
        assert!(matches!(page.events[0].kind, WorldEventKind::Input(_)));
        assert!(
            !format!("{:?}", page.events).contains("siesta"),
            "the schedule fire leaked to a joiner via events_after"
        );
    }

    #[tokio::test]
    async fn a_cancelled_schedule_never_fires_and_an_unknown_id_says_false() {
        let (mut actor, clock, token, _journal) =
            scheduling_actor("sched-cancel", &["schedule.at", "schedule.cancel"], false);
        let at = EPOCH_MS;
        // One pass: the cancel runs before any fire can fire the schedule.
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "soon", "at": at}},
                {"id": "c1", "cap": "schedule.cancel", "args": {"id": "soon"}},
                {"id": "c2", "cap": "schedule.cancel", "args": {"id": "ghost"}},
            ]}),
        )
        .await;
        assert_eq!(
            schedule_result(&actor, "s1").unwrap()["value"]["fires_at"],
            at
        );
        assert_eq!(
            schedule_result(&actor, "c1").unwrap()["value"]["cancelled"],
            true
        );
        assert_eq!(
            schedule_result(&actor, "c2").unwrap()["value"]["cancelled"],
            false
        );
        clock.advance(10_000);
        actor.fire_schedules().await;
        assert!(
            schedule_fires(&actor).is_empty(),
            "a cancelled schedule must not fire"
        );
    }

    #[tokio::test]
    async fn refused_schedule_requests_pend_nothing() {
        let (mut actor, _clock, token, journal) =
            scheduling_actor("sched-bad", &["schedule.at", "schedule.cancel"], false);
        let later = EPOCH_MS + 3_600_000;
        let far = EPOCH_MS + crate::world_caps::MAX_SCHEDULE_HORIZON_MS + 60_000;
        let long_id = "x".repeat(65);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "ok", "cap": "schedule.at", "args": {"id": "kept", "at": later}},
                {"id": "empty", "cap": "schedule.at", "args": {"id": "", "at": later}},
                {"id": "long", "cap": "schedule.at", "args": {"id": long_id, "at": later}},
                {"id": "far", "cap": "schedule.at", "args": {"id": "far", "at": far}},
                {"id": "dup", "cap": "schedule.at", "args": {"id": "kept", "at": later}},
                {"id": "missing", "cap": "schedule.at", "args": {"id": "nothing"}},
                {"id": "text", "cap": "schedule.at", "args": {"id": "t", "at": "soon"}},
                {"id": "bad-cancel", "cap": "schedule.cancel", "args": {"id": "a\nb"}},
            ]}),
        )
        .await;
        assert_eq!(schedule_result(&actor, "ok").unwrap()["ok"], true);
        for id in [
            "empty",
            "long",
            "far",
            "dup",
            "missing",
            "text",
            "bad-cancel",
        ] {
            assert_eq!(
                schedule_result(&actor, id).unwrap()["error"],
                "bad_args",
                "{id}"
            );
        }
        let pended: Vec<String> = journal
            .entries()
            .into_iter()
            .filter_map(|entry| match entry {
                JournalEntry::ScheduleSet { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(pended, ["kept"], "only the accepted request is journaled");
    }

    #[tokio::test]
    async fn a_fire_the_program_refuses_stays_pending_in_the_journal() {
        let (mut actor, clock, token, journal) =
            scheduling_actor("sched-refuse", &["schedule.at"], true);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "stuck", "at": EPOCH_MS}},
            ]}),
        )
        .await;
        assert!(schedule_result(&actor, "s1").is_some());
        clock.advance(1);
        actor.fire_schedules().await;
        // The program is unavailable now, but the actor still holds the state.
        assert!(!actor.program_healthy);

        let entries = journal.entries();
        assert!(
            !entries
                .iter()
                .any(|entry| matches!(entry, JournalEntry::ScheduleFired { .. })),
            "a refused fire is not consumed"
        );
        let journaled_fire = entries.iter().any(|entry| match entry {
            JournalEntry::System { event } => matches!(
                &event.kind,
                WorldEventKind::System { event } if event.contains("\"schedule\"")
            ),
            _ => false,
        });
        assert!(!journaled_fire, "a refused fire is not journaled");
        let restored = WorldCore::restore("sched-refuse", WorldLimits::default(), entries).unwrap();
        assert!(
            restored.core.schedules.contains_key("stuck"),
            "the next run delivers it"
        );
    }

    #[tokio::test]
    async fn a_schedule_overdue_at_restart_fires_late_on_the_first_wakeup() {
        let entries = vec![
            JournalEntry::Created {
                world_id: "sched-late".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::ScheduleSet {
                id: "overdue".to_owned(),
                at_ms: EPOCH_MS - 5_000,
                interval_ms: None,
                remaining: None,
                target: None,
            },
        ];
        let restored = WorldCore::restore("sched-late", WorldLimits::default(), entries).unwrap();
        let journal = Recorded::default();
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core = restored.core;
        core.clock = clock.clone();
        let mut actor = WorldActor::for_test(
            core,
            Box::new(Scheduler::default()),
            Some(Box::new(journal.clone())),
        );
        actor.fire_schedules().await;
        let fires = schedule_fires(&actor);
        assert_eq!(fires.len(), 1, "{fires:?}");
        assert_eq!(fires[0]["id"], "overdue");
        assert_eq!(fires[0]["late"], true);
        assert!(
            journal.entries().iter().any(
                |entry| matches!(entry, JournalEntry::ScheduleFired { id } if id == "overdue")
            )
        );
    }

    #[tokio::test]
    async fn a_pending_schedule_survives_a_restart_and_fires_on_time() {
        let at_ms = EPOCH_MS + 700;
        let entries = vec![
            JournalEntry::Created {
                world_id: "sched-later".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::ScheduleSet {
                id: "later".to_owned(),
                at_ms,
                interval_ms: None,
                remaining: None,
                target: None,
            },
        ];
        let restored = WorldCore::restore("sched-later", WorldLimits::default(), entries).unwrap();
        assert_eq!(
            restored.core.schedules.get("later"),
            Some(&PendingSchedule::one_shot(at_ms))
        );
        let clock: Arc<ManualClock> = Arc::new(ManualClock::new(EPOCH_MS));
        let mut core = restored.core;
        core.clock = clock.clone();
        let mut actor = WorldActor::for_test(core, Box::new(Scheduler::default()), None);
        actor.fire_schedules().await;
        assert!(schedule_fires(&actor).is_empty(), "not due yet");
        clock.advance(700);
        actor.fire_schedules().await;
        let fires = schedule_fires(&actor);
        assert_eq!(fires[0]["id"], "later");
        assert!(fires[0].get("late").is_none(), "it was not overdue at open");
    }

    #[tokio::test]
    async fn replay_reproduces_the_pending_and_fired_sets() {
        let (mut actor, clock, token, journal) =
            scheduling_actor("sched-replay", &["schedule.at", "schedule.cancel"], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "keep", "at": EPOCH_MS + 3_600_000}},
                {"id": "s2", "cap": "schedule.at", "args": {"id": "now", "at": EPOCH_MS}},
                {"id": "s3", "cap": "schedule.at", "args": {"id": "gone", "at": EPOCH_MS + 3_600_000}},
                {"id": "c3", "cap": "schedule.cancel", "args": {"id": "gone"}},
            ]}),
        )
        .await;
        clock.advance(1);
        actor.fire_schedules().await;
        let live_fired: Vec<String> = schedule_fires(&actor)
            .iter()
            .filter_map(|fire| fire["id"].as_str().map(str::to_owned))
            .collect();

        let restored =
            WorldCore::restore("sched-replay", WorldLimits::default(), journal.entries()).unwrap();
        let pending: Vec<String> = restored.core.schedules.keys().cloned().collect();
        assert_eq!(pending, ["keep"]);
        let replayed_fired: Vec<String> = restored
            .replay
            .iter()
            .filter_map(|event| match &event.kind {
                WorldEventKind::System { event } => {
                    serde_json::from_str::<serde_json::Value>(event)
                        .ok()
                        .filter(|value| value["cap"] == "schedule")
                        .and_then(|value| value["id"].as_str().map(str::to_owned))
                }
                _ => None,
            })
            .collect();
        assert_eq!(live_fired, ["now"]);
        assert_eq!(replayed_fired, live_fired);
    }

    #[test]
    fn a_checkpoint_keeps_pending_schedules_and_writes_their_entries() {
        let now = unix_ms();
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.schedules
            .insert("a".to_owned(), PendingSchedule::one_shot(now + 100));
        core.schedules
            .insert("b".to_owned(), PendingSchedule::one_shot(now + 200));
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        for (id, at_ms) in [("a", now + 100), ("b", now + 200)] {
            let set = JournalEntry::ScheduleSet {
                id: id.to_owned(),
                at_ms,
                interval_ms: None,
                remaining: None,
                target: None,
            };
            assert!(
                entries.contains(&set),
                "a compacted journal keeps the schedule {id}"
            );
        }
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.schedules, core.schedules);
    }

    #[test]
    fn a_checkpoint_that_disagrees_with_its_schedule_entries_is_refused() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        };
        let set = || JournalEntry::ScheduleSet {
            id: "a".to_owned(),
            at_ms: 5,
            interval_ms: None,
            remaining: None,
            target: None,
        };
        let checkpoint = |schedules: Option<Vec<CheckpointSchedule>>| JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules,
            kv: None,
            blobs: None,
            transfers: None,
            lease: None,
            snapshot: "s".to_owned(),
            events: Vec::new(),
            admin_history: None,
            open_presence: None,
        };
        let declared = || CheckpointSchedule {
            id: "a".to_owned(),
            at_ms: 5,
            interval_ms: None,
            remaining: 1,
            target: None,
        };
        let agreed = [header(), set(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), agreed).is_ok());
        let omitted = [header(), set(), checkpoint(None)];
        assert!(WorldCore::restore("w", WorldLimits::default(), omitted).is_err());
        let unwritten = [header(), checkpoint(Some(vec![declared()]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), unwritten).is_err());
        let duplicated = [
            header(),
            set(),
            checkpoint(Some(vec![declared(), declared()])),
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), duplicated).is_err());
    }

    #[test]
    fn a_schedule_entry_must_refer_to_a_pending_schedule() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION,
        };
        let fired = [header(), JournalEntry::ScheduleFired { id: "x".to_owned() }];
        assert!(WorldCore::restore("w", WorldLimits::default(), fired).is_err());
        let cancelled = [
            header(),
            JournalEntry::ScheduleCancelled { id: "x".to_owned() },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), cancelled).is_err());
        let twice = [
            header(),
            JournalEntry::ScheduleSet {
                id: "x".to_owned(),
                at_ms: 1,
                interval_ms: None,
                remaining: None,
                target: None,
            },
            JournalEntry::ScheduleSet {
                id: "x".to_owned(),
                at_ms: 2,
                interval_ms: None,
                remaining: None,
                target: None,
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), twice).is_err());
        let malformed = [
            header(),
            JournalEntry::ScheduleSet {
                id: "a\nb".to_owned(),
                at_ms: 1,
                interval_ms: None,
                remaining: None,
                target: None,
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), malformed).is_err());
    }

    #[test]
    fn schedules_wake_the_loop_only_while_the_program_can_take_them() {
        let mut schedules = BTreeMap::new();
        schedules.insert("a".to_owned(), PendingSchedule::one_shot(2_000));
        let now = std::time::Instant::now();
        assert!(next_schedule_wake(&schedules, 1_000, now, true, None).is_some());
        assert!(
            next_schedule_wake(&schedules, 3_000, now, true, None).is_some(),
            "an overdue schedule wakes the loop now"
        );
        assert!(
            next_schedule_wake(&schedules, 3_000, now, false, None).is_none(),
            "a program that refuses fires never wakes the loop for one"
        );
        assert!(next_schedule_wake(&BTreeMap::new(), 1_000, now, true, None).is_none());
        // #1067: a due schedule whose fire was refused at the fuel rate
        // waits until the retry deadline instead of waking the loop now.
        assert_eq!(
            next_schedule_wake(
                &schedules,
                3_000,
                now,
                true,
                Some(now + Duration::from_secs(1))
            ),
            Some(now + Duration::from_secs(1)),
        );
        // A schedule that is not due yet keeps its own wake, and a
        // spent retry deadline holds nothing back.
        assert_eq!(
            next_schedule_wake(
                &schedules,
                1_000,
                now,
                true,
                Some(now + Duration::from_secs(1))
            ),
            Some(now + Duration::from_millis(1_000)),
        );
        assert_eq!(
            next_schedule_wake(
                &schedules,
                3_000,
                now,
                true,
                Some(now - Duration::from_secs(1))
            ),
            Some(now),
        );
    }

    #[test]
    fn schedule_entries_keep_their_journal_tags() {
        let set = serde_json::to_value(JournalEntry::ScheduleSet {
            id: "a".to_owned(),
            at_ms: 7,
            interval_ms: None,
            remaining: None,
            target: None,
        })
        .unwrap();
        assert_eq!(
            set,
            serde_json::json!({"entry": "schedule_set", "id": "a", "at_ms": 7})
        );
        let cancelled =
            serde_json::to_value(JournalEntry::ScheduleCancelled { id: "a".to_owned() }).unwrap();
        assert_eq!(cancelled["entry"], "schedule_cancelled");
        let fired =
            serde_json::to_value(JournalEntry::ScheduleFired { id: "a".to_owned() }).unwrap();
        assert_eq!(fired["entry"], "schedule_fired");
    }

    /// `schedule.at` returns the reason a schedule was refused, not a bare
    /// `bad_args`: the program (and any operator reading the result) can see
    /// why, on the wire.
    #[tokio::test]
    async fn a_refused_schedule_carries_its_reason_on_the_wire() {
        let (mut actor, _clock, token, _journal) =
            scheduling_actor("sched-reason", &["schedule.at"], false);
        // First request is accepted; the second, reusing the id, is refused by
        // `check_schedule` with a specific reason.
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "ok", "cap": "schedule.at", "args": {"id": "nap", "at": EPOCH_MS + 60_000}},
            ]}),
        )
        .await;
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "dup", "cap": "schedule.at", "args": {"id": "nap", "at": EPOCH_MS + 120_000}},
            ]}),
        )
        .await;
        let result = schedule_result(&actor, "dup").unwrap();
        assert_eq!(result["error"], "bad_args");
        let reason = result["reason"]
            .as_str()
            .expect("a reason is carried on the wire");
        assert!(
            reason.contains("already pending"),
            "unexpected reason: {reason}"
        );
    }

    /// #1355: a recurring `schedule.at` fires its exact finite count,
    /// fixed-rate — a clock jump past every planned time neither drifts
    /// the series nor extends it, and the schedule is gone afterwards.
    #[tokio::test]
    async fn a_recurring_schedule_fires_its_exact_count_then_stops() {
        let (mut actor, clock, token, _journal) =
            scheduling_actor("sched-recur", &["schedule.at"], false);
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "set", "cap": "schedule.at", "args": {"id": "bell", "at": EPOCH_MS + 100, "interval_ms": 100, "count": 3}},
            ]}),
        )
        .await;
        let result = schedule_result(&actor, "set").unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["value"]["count"], 3, "{result}");
        assert_eq!(result["value"]["interval_ms"], 100, "{result}");
        actor.fire_schedules().await;
        assert!(schedule_fires(&actor).is_empty(), "not due yet");
        clock.advance(10_000);
        for _ in 0..3 {
            actor.fire_schedules().await;
        }
        let fires = schedule_fires(&actor);
        assert_eq!(fires.len(), 3, "{fires:?}");
        assert!(fires.iter().all(|fire| fire["id"] == "bell"));
        actor.fire_schedules().await;
        assert_eq!(schedule_fires(&actor).len(), 3, "the series is exhausted");
        assert!(actor.core.schedules.is_empty());
    }

    /// #1355: replay derives a series' mid-state from its journaled fires
    /// alone — no wall clock — and the version gate names recurrence.
    #[test]
    fn recurrence_replay_advances_from_the_journaled_fires() {
        // A journal holding a recurring set carries the recurrence header
        // version: the store bumps it on the first such append.
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_SCHEDULE_RECURRENCE,
        };
        let set = JournalEntry::ScheduleSet {
            id: "r".to_owned(),
            at_ms: 1_000,
            interval_ms: Some(100),
            remaining: Some(3),
            target: None,
        };
        let fired = || JournalEntry::ScheduleFired { id: "r".to_owned() };
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [header(), set.clone(), fired()],
        )
        .unwrap();
        assert_eq!(
            restored.core.schedules["r"],
            PendingSchedule {
                at_ms: 1_100,
                interval_ms: Some(100),
                remaining: 2,
                target: None,
            }
        );
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [header(), set.clone(), fired(), fired()],
        )
        .unwrap();
        assert_eq!(
            restored.core.schedules["r"],
            PendingSchedule {
                at_ms: 1_200,
                interval_ms: Some(100),
                remaining: 1,
                target: None,
            }
        );
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [header(), set.clone(), fired(), fired(), fired()],
        )
        .unwrap();
        assert!(!restored.core.schedules.contains_key("r"));
        assert_eq!(
            journal_version_required_by(&set),
            JOURNAL_VERSION_SCHEDULE_RECURRENCE
        );
        let one_shot = JournalEntry::ScheduleSet {
            id: "a".to_owned(),
            at_ms: 7,
            interval_ms: None,
            remaining: None,
            target: None,
        };
        assert_eq!(journal_version_required_by(&one_shot), JOURNAL_VERSION);
    }

    /// #1355: a recurring schedule survives checkpoint compaction with its
    /// interval and remaining count, and the checkpoint requires version 8.
    #[test]
    fn a_recurring_schedule_survives_checkpoint_compaction() {
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.schedules.insert(
            "r".to_owned(),
            PendingSchedule {
                at_ms: 500,
                interval_ms: Some(50),
                remaining: 4,
                target: None,
            },
        );
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        assert!(
            entries.iter().any(|entry| {
                journal_version_required_by(entry) == JOURNAL_VERSION_SCHEDULE_RECURRENCE
            }),
            "a checkpoint carrying a series requires the recurrence version"
        );
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.schedules, core.schedules);
    }

    /// #1355: re-arming changes a schedule in place — one cap slot however
    /// many fires remain.
    #[test]
    fn a_recurring_schedule_holds_one_cap_slot_across_rearms() {
        let now = 1_000_000_000_000u64;
        let mut core = world(8);
        for index in 0..crate::world_caps::MAX_PENDING_SCHEDULES - 1 {
            core.schedules
                .insert(format!("s{index}"), PendingSchedule::one_shot(now));
        }
        core.schedules.insert(
            "r".to_owned(),
            PendingSchedule {
                at_ms: now,
                interval_ms: Some(10),
                remaining: 5,
                target: None,
            },
        );
        assert!(core.apply_schedule_fired("r"));
        assert_eq!(
            core.schedules.len(),
            crate::world_caps::MAX_PENDING_SCHEDULES
        );
        assert_eq!(core.schedules["r"].remaining, 4);
        assert_eq!(core.schedules["r"].at_ms, now + 10);
        let error = core.check_schedule("one-more", now, now).unwrap_err();
        assert!(error.to_string().contains("pending schedules"), "{error}");
    }

    /// #1355: the recurrence pair's bounds, and the whole series inside
    /// the horizon, are refused with reasons before anything is journaled.
    #[test]
    fn recurring_schedule_bounds_are_refused_with_reasons() {
        let now = 1_000_000_000_000u64;
        let horizon = crate::world_caps::MAX_SCHEDULE_HORIZON_MS;
        let core = world(8);
        let error = core
            .check_schedule_recurring("a", now + 100, None, Some(3), now)
            .unwrap_err();
        assert!(error.to_string().contains("interval_ms"), "{error}");
        let error = core
            .check_schedule_recurring("a", now + 100, Some(100), None, now)
            .unwrap_err();
        assert!(error.to_string().contains("finite count"), "{error}");
        let error = core
            .check_schedule_recurring("a", now + 100, Some(100), Some(1), now)
            .unwrap_err();
        assert!(error.to_string().contains("count must be 2"), "{error}");
        let error = core
            .check_schedule_recurring("a", now + horizon - 50, Some(100), Some(3), now)
            .unwrap_err();
        assert!(error.to_string().contains("series ends"), "{error}");
        assert!(
            core.check_schedule_recurring("a", now + 100, Some(100), Some(3), now)
                .is_ok()
        );
    }

    /// #1355: recurrence keeps its journal shape, legacy entries parse as
    /// one-shot, and checkpoint records round-trip in both forms.
    #[test]
    fn schedule_recurrence_keeps_its_journal_shape_and_legacy_entries_parse() {
        let recurring = serde_json::to_value(JournalEntry::ScheduleSet {
            id: "r".to_owned(),
            at_ms: 7,
            interval_ms: Some(50),
            remaining: Some(3),
            target: None,
        })
        .unwrap();
        assert_eq!(
            recurring,
            serde_json::json!({"entry": "schedule_set", "id": "r", "at_ms": 7, "interval_ms": 50, "remaining": 3})
        );
        let legacy: JournalEntry = serde_json::from_value(
            serde_json::json!({"entry": "schedule_set", "id": "a", "at_ms": 7}),
        )
        .unwrap();
        assert_eq!(
            legacy,
            JournalEntry::ScheduleSet {
                id: "a".to_owned(),
                at_ms: 7,
                interval_ms: None,
                remaining: None,
                target: None
            }
        );
        let pair: CheckpointSchedule = serde_json::from_value(serde_json::json!(["a", 5])).unwrap();
        assert_eq!(
            pair,
            CheckpointSchedule {
                id: "a".to_owned(),
                at_ms: 5,
                interval_ms: None,
                remaining: 1,
                target: None,
            }
        );
        assert_eq!(
            serde_json::to_value(&pair).unwrap(),
            serde_json::json!(["a", 5])
        );
        let full = CheckpointSchedule {
            id: "r".to_owned(),
            at_ms: 7,
            interval_ms: Some(50),
            remaining: 3,
            target: None,
        };
        let full_json = serde_json::to_value(&full).unwrap();
        assert_eq!(
            full_json,
            serde_json::json!({"id": "r", "at_ms": 7, "interval_ms": 50, "remaining": 3})
        );
        assert_eq!(
            serde_json::from_value::<CheckpointSchedule>(full_json).unwrap(),
            full
        );
    }

    /// #1355 part 2: the targeted check refuses, with reasons, a target
    /// that is this world, a malformed name, an isolated source, and a
    /// peer the admin has not granted send consent for.
    #[test]
    fn cross_world_targets_are_refused_without_reciprocal_setup() {
        let now = 1_000_000_000_000u64;
        let mut core = world(8);
        // Isolated by default: even a granted peer is refused.
        core.schedule_grants.insert(
            "dst-w".to_owned(),
            ScheduleGrant {
                send: true,
                receive: false,
            },
        );
        let error = core
            .check_schedule_targeted("x", now + 100, None, None, Some("dst-w"), now)
            .unwrap_err();
        assert!(error.to_string().contains("isolated"), "{error}");
        core.isolation = Isolation::Unisolated;
        let error = core
            .check_schedule_targeted("x", now + 100, None, None, Some("test-world"), now)
            .unwrap_err();
        assert!(error.to_string().contains("its own world"), "{error}");
        let error = core
            .check_schedule_targeted("x", now + 100, None, None, Some("Bad Name"), now)
            .unwrap_err();
        assert!(
            error.to_string().contains("invalid schedule target"),
            "{error}"
        );
        let error = core
            .check_schedule_targeted("x", now + 100, None, None, Some("other-w"), now)
            .unwrap_err();
        assert!(error.to_string().contains("no send grant"), "{error}");
        assert!(
            core.check_schedule_targeted("x", now + 100, None, None, Some("dst-w"), now)
                .is_ok()
        );
    }

    /// #1355 part 2: a due cross-world schedule fires through the sink,
    /// and only the sink's durable acceptance consumes the occurrence.
    #[tokio::test]
    async fn a_cross_world_fire_is_consumed_only_on_sink_acceptance() {
        struct RecordingSink {
            calls: std::sync::Mutex<Vec<(String, String, String, u64)>>,
            accept: bool,
        }
        impl ScheduleDelivery for RecordingSink {
            fn deliver_schedule<'a>(
                &'a self,
                source: &'a str,
                target: &'a str,
                id: &'a str,
                at_ms: u64,
                _late: bool,
            ) -> ScheduleDeliveryFuture<'a> {
                Box::pin(async move {
                    self.calls.lock().unwrap().push((
                        source.to_owned(),
                        target.to_owned(),
                        id.to_owned(),
                        at_ms,
                    ));
                    self.accept
                })
            }
        }
        let (mut actor, _clock, _token, _journal) =
            scheduling_actor("src-w", &["schedule.at"], false);
        actor.core.isolation = Isolation::Unisolated;
        actor.core.schedule_grants.insert(
            "dst-w".to_owned(),
            ScheduleGrant {
                send: true,
                receive: false,
            },
        );
        actor.core.schedules.insert(
            "x".to_owned(),
            PendingSchedule {
                at_ms: EPOCH_MS - 1,
                interval_ms: None,
                remaining: 1,
                target: Some("dst-w".to_owned()),
            },
        );
        // No sink installed: the occurrence stays pending.
        actor.fire_schedules().await;
        assert!(actor.core.schedules.contains_key("x"));
        let sink = std::sync::Arc::new(RecordingSink {
            calls: std::sync::Mutex::new(Vec::new()),
            accept: false,
        });
        actor.schedule_delivery = Some(sink.clone());
        actor.fire_schedules().await;
        assert_eq!(sink.calls.lock().unwrap().len(), 1, "one delivery attempt");
        assert!(
            actor.core.schedules.contains_key("x"),
            "a refused delivery consumes nothing"
        );
        let sink = std::sync::Arc::new(RecordingSink {
            calls: std::sync::Mutex::new(Vec::new()),
            accept: true,
        });
        actor.schedule_delivery = Some(sink.clone());
        actor.fire_schedules().await;
        assert_eq!(
            sink.calls.lock().unwrap()[0],
            (
                "src-w".to_owned(),
                "dst-w".to_owned(),
                "x".to_owned(),
                EPOCH_MS - 1
            )
        );
        assert!(
            !actor.core.schedules.contains_key("x"),
            "acceptance consumes the occurrence"
        );
    }

    /// #1355 part 2: the target accepts only with a receive grant while
    /// unisolated, and a receipt answers a retry without a second fire.
    #[tokio::test]
    async fn cross_world_acceptance_needs_a_receive_grant_and_receipts_dedupe() {
        let (mut actor, _clock, _token, _journal) = scheduling_actor("dst-w", &[], false);
        async fn deliver(actor: &mut WorldActor, at_ms: u64) -> bool {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::DeliverSchedule {
                    source: "src-w".to_owned(),
                    id: "x".to_owned(),
                    at_ms,
                    late: false,
                    reply,
                })
                .await;
            response.await.unwrap()
        }
        // Isolated (the default): refused even before grants are read.
        assert!(!deliver(&mut actor, EPOCH_MS).await);
        actor.core.isolation = Isolation::Unisolated;
        // Unisolated but no receive grant: refused.
        assert!(!deliver(&mut actor, EPOCH_MS).await);
        actor.core.schedule_grants.insert(
            "src-w".to_owned(),
            ScheduleGrant {
                send: false,
                receive: true,
            },
        );
        assert!(deliver(&mut actor, EPOCH_MS).await);
        let fires = schedule_fires(&actor);
        assert_eq!(fires.len(), 1, "{fires:?}");
        assert_eq!(fires[0]["id"], "x");
        assert_eq!(fires[0]["from"], "src-w");
        // A retry of the same occurrence is answered from the receipt.
        assert!(deliver(&mut actor, EPOCH_MS).await);
        assert_eq!(schedule_fires(&actor).len(), 1, "no second delivery");
        // The next occurrence (a later planned time) is a new fire.
        assert!(deliver(&mut actor, EPOCH_MS + 100).await);
        assert_eq!(schedule_fires(&actor).len(), 2);
    }

    /// #1355 part 2: `schedule.at` with a `world` names the missing grant
    /// on the wire, and pends the target once an admin grants send.
    #[tokio::test]
    async fn a_cross_world_schedule_at_names_the_missing_grant_then_pends() {
        let (mut actor, _clock, token, _journal) =
            scheduling_actor("src-w", &["schedule.at"], false);
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::SetIsolation {
                authority: Authority::Admin,
                isolation: Isolation::Unisolated,
                reply,
            })
            .await;
        response.await.unwrap().unwrap();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "x1", "cap": "schedule.at", "args": {"id": "bell", "at": EPOCH_MS + 60_000, "world": "dst-w"}},
            ]}),
        )
        .await;
        let result = schedule_result(&actor, "x1").unwrap();
        assert_eq!(result["error"], "bad_args");
        let reason = result["reason"].as_str().expect("a reason is carried");
        assert!(
            reason.contains("no send grant"),
            "unexpected reason: {reason}"
        );
        let (reply, response) = tokio::sync::oneshot::channel();
        actor
            .handle(WorldCommand::AdminSetScheduleGrant {
                peer: "dst-w".to_owned(),
                send: true,
                receive: false,
                reply,
            })
            .await;
        response.await.unwrap().unwrap();
        ask(
            &mut actor,
            &token,
            serde_json::json!({"requests": [
                {"id": "x2", "cap": "schedule.at", "args": {"id": "bell", "at": EPOCH_MS + 60_000, "world": "dst-w"}},
            ]}),
        )
        .await;
        let result = schedule_result(&actor, "x2").unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["value"]["world"], "dst-w", "{result}");
        assert_eq!(
            actor.core.schedules["bell"].target.as_deref(),
            Some("dst-w")
        );
    }

    /// #1355 part 2: grants and receipts replay from the journal, a
    /// revocation clears the grant, and both survive compaction.
    #[test]
    fn schedule_grants_and_receipts_replay_and_survive_compaction() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_SCHEDULE_RECURRENCE,
        };
        let grant = JournalEntry::ScheduleGrantSet {
            peer: "dst-w".to_owned(),
            send: true,
            receive: false,
        };
        let receipt = JournalEntry::ScheduleReceived {
            source: "src-w".to_owned(),
            id: "x".to_owned(),
            at_ms: 5,
        };
        assert_eq!(
            journal_version_required_by(&grant),
            JOURNAL_VERSION_SCHEDULE_RECURRENCE
        );
        assert_eq!(
            journal_version_required_by(&receipt),
            JOURNAL_VERSION_SCHEDULE_RECURRENCE
        );
        let restored = WorldCore::restore(
            "w",
            WorldLimits::default(),
            [header(), grant.clone(), receipt],
        )
        .unwrap();
        assert_eq!(
            restored.core.schedule_grants["dst-w"],
            ScheduleGrant {
                send: true,
                receive: false
            }
        );
        assert!(
            restored
                .core
                .schedule_receipts
                .contains(&("src-w".to_owned(), "x".to_owned(), 5))
        );
        let revoked = JournalEntry::ScheduleGrantSet {
            peer: "dst-w".to_owned(),
            send: false,
            receive: false,
        };
        let restored =
            WorldCore::restore("w", WorldLimits::default(), [header(), grant, revoked]).unwrap();
        assert!(restored.core.schedule_grants.is_empty());
        // Compaction: a core carrying both re-emits them before its
        // checkpoint, so a restored core sees the same state.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.schedule_grants.insert(
            "dst-w".to_owned(),
            ScheduleGrant {
                send: true,
                receive: false,
            },
        );
        core.schedule_receipts
            .insert(("src-w".to_owned(), "x".to_owned(), 5));
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.schedule_grants, core.schedule_grants);
        assert_eq!(restored.core.schedule_receipts, core.schedule_receipts);
    }
    // ---- F12.14 portals wiring (code#1068) ----

    /// A program that records every host (system) event it receives,
    /// so a test can see exactly what a portal delivery handed it.
    struct PortalRecorder {
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl WorldProgram for PortalRecorder {
        fn update(&mut self, event: &WorldEvent) -> Result<()> {
            if let WorldEventKind::System { event } = &event.kind {
                self.seen.lock().unwrap().push(event.clone());
            }
            Ok(())
        }

        fn wants(&mut self) -> Result<Option<String>> {
            Ok(None)
        }
    }

    /// The test transport: a switchboard over a fixed set of handles,
    /// standing in for `HubPortalDelivery` exactly as `RecordingSink`
    /// stands in for `HubScheduleDelivery` in the schedule tests.
    struct PortalSwitchboard {
        handles: Arc<Mutex<BTreeMap<String, WorldHandle>>>,
    }

    impl PortalSwitchboard {
        fn handle(&self, world: &str) -> Option<WorldHandle> {
            self.handles.lock().unwrap().get(world).cloned()
        }
    }

    impl PortalDelivery for PortalSwitchboard {
        fn peer_portal_grants<'a>(
            &'a self,
            world: &'a str,
        ) -> PortalDeliveryFuture<'a, Option<PortalGrants>> {
            Box::pin(async move {
                let handle = self.handle(world)?;
                handle.portal_grants().await.ok()
            })
        }

        fn deliver_portal<'a>(
            &'a self,
            source: &'a str,
            target: &'a str,
            message: &'a str,
        ) -> PortalDeliveryFuture<'a, Result<(), PortalSendError>> {
            Box::pin(async move {
                let Some(handle) = self.handle(target) else {
                    return Err(PortalSendError::Unavailable(format!(
                        "unknown portal target {target}"
                    )));
                };
                handle
                    .deliver_portal_message(source.to_owned(), message.to_owned())
                    .await
            })
        }
    }

    /// Two spawned worlds wired to each other through a switchboard,
    /// plus the recorders watching what each program received.
    async fn portal_pair() -> (
        WorldHandle,
        WorldHandle,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let seen_a = Arc::new(Mutex::new(Vec::new()));
        let seen_b = Arc::new(Mutex::new(Vec::new()));
        let a = WorldHandle::spawn_with_program(
            WorldCore::new("world-a", WorldLimits::default()).unwrap(),
            Box::new(PortalRecorder {
                seen: Arc::clone(&seen_a),
            }),
        );
        let b = WorldHandle::spawn_with_program(
            WorldCore::new("world-b", WorldLimits::default()).unwrap(),
            Box::new(PortalRecorder {
                seen: Arc::clone(&seen_b),
            }),
        );
        let switchboard = Arc::new(PortalSwitchboard {
            handles: Arc::new(Mutex::new(BTreeMap::from([
                ("world-a".to_owned(), a.clone()),
                ("world-b".to_owned(), b.clone()),
            ]))),
        });
        a.set_portal_delivery(switchboard.clone()).await.unwrap();
        b.set_portal_delivery(switchboard).await.unwrap();
        (a, b, seen_a, seen_b)
    }

    async fn grant_both(a: &WorldHandle, b: &WorldHandle) {
        a.admin_set_portal_grant("world-b".to_owned(), true)
            .await
            .unwrap();
        b.admin_set_portal_grant("world-a".to_owned(), true)
            .await
            .unwrap();
    }

    fn portal_audit(handle: &WorldHandle, action: &str) -> Vec<AuditRecord> {
        handle
            .audit_records()
            .into_iter()
            .filter(|record| record.action == action)
            .collect()
    }

    /// Oracle (F12.14 exit proof, wiring slice): a granted pair
    /// exchanges a message — the receiver's program gets a host event
    /// naming the sender, and EACH world's audit log carries exactly
    /// one portal entry for the exchange, send on the sender and
    /// deliver on the receiver. The admin grants themselves are
    /// audit-recorded like every other admin command.
    #[tokio::test]
    async fn portal_granted_pair_exchanges_with_audit_on_both_sides() {
        let (a, b, _seen_a, seen_b) = portal_pair().await;
        grant_both(&a, &b).await;
        a.send_portal_message("world-b".to_owned(), "hello through the portal".to_owned())
            .await
            .unwrap();
        let seen = seen_b.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&seen[0]).unwrap();
        assert_eq!(event["cap"], "portal");
        assert_eq!(event["event"], "message");
        assert_eq!(event["from"], "world-a");
        assert_eq!(event["message"], "hello through the portal");

        let sends = portal_audit(&a, "portal.send");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].outcome, "accepted");
        assert_eq!(sends[0].parameters["from_world"], "world-a");
        assert_eq!(sends[0].parameters["to_world"], "world-b");
        assert_eq!(sends[0].parameters["bytes"], 24);
        let delivers = portal_audit(&b, "portal.deliver");
        assert_eq!(delivers.len(), 1);
        assert_eq!(delivers[0].outcome, "accepted");
        assert_eq!(delivers[0].parameters, sends[0].parameters);
        assert!(portal_audit(&b, "portal.send").is_empty());
        assert!(portal_audit(&a, "portal.deliver").is_empty());

        for handle in [&a, &b] {
            let grants = portal_audit(handle, "AdminSetPortalGrant");
            assert_eq!(grants.len(), 1);
            assert_eq!(grants[0].outcome, "accepted");
            assert_eq!(grants[0].parameters["granted"], true);
        }
    }

    /// Oracle: a sender that never granted the portal refuses before
    /// any transport happens — the refusal is the kernel's
    /// sender-not-granted, audited on the sender, and the peer sees
    /// nothing at all.
    #[tokio::test]
    async fn portal_sender_ungranted_refuses_with_sender_audit() {
        let (a, b, _seen_a, seen_b) = portal_pair().await;
        b.admin_set_portal_grant("world-a".to_owned(), true)
            .await
            .unwrap();
        let error = a
            .send_portal_message("world-b".to_owned(), "anyone there?".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::SenderNotGranted {
                peer: "world-b".to_owned()
            })
        );
        let sends = portal_audit(&a, "portal.send");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].outcome, "refused");
        assert_eq!(sends[0].parameters["refusal"], "portal.sender_not_granted");
        assert!(portal_audit(&b, "portal.deliver").is_empty());
        assert!(seen_b.lock().unwrap().is_empty());
    }

    /// Oracle: a granted sender whose peer never granted back refuses
    /// as peer-not-granted — the mutual check reads BOTH books.
    #[tokio::test]
    async fn portal_peer_ungranted_refuses() {
        let (a, b, _seen_a, seen_b) = portal_pair().await;
        a.admin_set_portal_grant("world-b".to_owned(), true)
            .await
            .unwrap();
        let error = a
            .send_portal_message("world-b".to_owned(), "knock knock".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::PeerNotGranted {
                peer: "world-a".to_owned()
            })
        );
        let sends = portal_audit(&a, "portal.send");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].parameters["refusal"], "portal.peer_not_granted");
        assert!(portal_audit(&b, "portal.deliver").is_empty());
        assert!(seen_b.lock().unwrap().is_empty());
    }

    /// Oracle (PR #10840 CodeRabbit regression): simultaneous
    /// sends in both directions complete - the send orchestration
    /// runs outside the actor command loops, so neither actor
    /// waits on the other while holding its own loop.
    #[tokio::test]
    async fn portal_bidirectional_sends_do_not_deadlock() {
        let (a, b, seen_a, seen_b) = portal_pair().await;
        grant_both(&a, &b).await;
        let both = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            tokio::join!(
                a.send_portal_message("world-b".to_owned(), "ping".to_owned()),
                b.send_portal_message("world-a".to_owned(), "pong".to_owned())
            )
        })
        .await
        .expect("bidirectional portal sends must not deadlock");
        both.0.unwrap();
        both.1.unwrap();
        assert_eq!(seen_b.lock().unwrap().len(), 1);
        assert_eq!(seen_a.lock().unwrap().len(), 1);
    }

    /// Oracle (PR #10840 CodeRabbit regression): a world can send
    /// a portal message to itself - its own grant book is both
    /// halves of the mutual check, and the delivery is processed
    /// because the send does not hold the actor loop while waiting.
    #[tokio::test]
    async fn portal_self_send_delivers() {
        let (a, _b, seen_a, _seen_b) = portal_pair().await;
        a.admin_set_portal_grant("world-a".to_owned(), true)
            .await
            .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            a.send_portal_message("world-a".to_owned(), "note to self".to_owned()),
        )
        .await
        .expect("self portal send must not deadlock")
        .unwrap();
        let seen = seen_a.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        let event: serde_json::Value = serde_json::from_str(&seen[0]).unwrap();
        assert_eq!(event["from"], "world-a");
        assert_eq!(event["message"], "note to self");
    }

    /// Oracle (PR #10840 CodeRabbit regression): a peer whose
    /// grant book cannot be read (unknown to the transport) is a
    /// transport failure - `Unavailable`, with NO portal.send audit
    /// entry, because no policy decision was ever made. Contrast
    /// `portal_peer_ungranted_refuses`, where the book IS read and
    /// the refusal is audited.
    #[tokio::test]
    async fn portal_unreadable_peer_book_is_unavailable_not_refused() {
        let (a, _b, _seen_a, _seen_b) = portal_pair().await;
        a.admin_set_portal_grant("world-c".to_owned(), true)
            .await
            .unwrap();
        let error = a
            .send_portal_message("world-c".to_owned(), "anyone there?".to_owned())
            .await
            .unwrap_err();
        assert!(
            matches!(error, PortalSendError::Unavailable(_)),
            "unreadable peer book must be Unavailable, got {error:?}"
        );
        assert!(portal_audit(&a, "portal.send").is_empty());
    }

    /// Oracle (PR #10840 CodeRabbit regression): a granted send
    /// with NO transport installed is `Unavailable`, not a
    /// peer-not-granted refusal - no peer book exists to refuse on.
    #[tokio::test]
    async fn portal_granted_send_without_sink_is_unavailable() {
        let handle =
            WorldHandle::spawn(WorldCore::new("world-solo", WorldLimits::default()).unwrap());
        handle
            .admin_set_portal_grant("world-b".to_owned(), true)
            .await
            .unwrap();
        let error = handle
            .send_portal_message("world-b".to_owned(), "hello?".to_owned())
            .await
            .unwrap_err();
        assert!(
            matches!(error, PortalSendError::Unavailable(_)),
            "sinkless send must be Unavailable, got {error:?}"
        );
        assert!(portal_audit(&handle, "portal.send").is_empty());
    }

    /// Oracle (PR #10840 CodeRabbit regression): a message at the
    /// raw 16 KiB bound made of control characters passes the raw
    /// check, but its serialized host event escapes past
    /// MAX_EVENT_BYTES - it refuses as message-too-large carrying
    /// the envelope size and the event ceiling, on the send side,
    /// instead of being dropped by delivery as Unavailable.
    #[tokio::test]
    async fn portal_escapable_message_refuses_on_envelope_bound() {
        let (a, b, _seen_a, seen_b) = portal_pair().await;
        grant_both(&a, &b).await;
        let message = "\0".repeat(id_caps::portal::MAX_PORTAL_MESSAGE_BYTES);
        assert_eq!(message.len(), id_caps::portal::MAX_PORTAL_MESSAGE_BYTES);
        let error = a
            .send_portal_message("world-b".to_owned(), message.clone())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::MessageTooLarge {
                bytes: portal_message_event("world-a", &message).len(),
                max: crate::world_caps::MAX_EVENT_BYTES,
            })
        );
        let sends = portal_audit(&a, "portal.send");
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].parameters["refusal"], "portal.message_too_large");
        assert!(portal_audit(&b, "portal.deliver").is_empty());
        assert!(seen_b.lock().unwrap().is_empty());
    }
    /// Oracle: revoking mid-conversation refuses the next message as
    /// revoked — distinct from never-granted — on whichever side
    /// revoked: the sender's own revoke refuses as sender-revoked,
    /// and after a re-grant the peer's revoke refuses as
    /// peer-revoked.
    #[tokio::test]
    async fn portal_revoke_mid_conversation_refuses_as_revoked() {
        let (a, b, _seen_a, _seen_b) = portal_pair().await;
        grant_both(&a, &b).await;
        a.send_portal_message("world-b".to_owned(), "first".to_owned())
            .await
            .unwrap();
        a.admin_set_portal_grant("world-b".to_owned(), false)
            .await
            .unwrap();
        let error = a
            .send_portal_message("world-b".to_owned(), "second".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::SenderRevoked {
                peer: "world-b".to_owned()
            })
        );
        a.admin_set_portal_grant("world-b".to_owned(), true)
            .await
            .unwrap();
        b.admin_set_portal_grant("world-a".to_owned(), false)
            .await
            .unwrap();
        let error = a
            .send_portal_message("world-b".to_owned(), "third".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::PeerRevoked {
                peer: "world-a".to_owned()
            })
        );
        let refusals: Vec<String> = portal_audit(&a, "portal.send")
            .iter()
            .filter(|record| record.outcome == "refused")
            .map(|record| record.parameters["refusal"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            refusals,
            [
                "portal.sender_revoked".to_owned(),
                "portal.peer_revoked".to_owned()
            ]
        );
    }

    /// Oracle: the bounds refuse with their own codes — one byte over
    /// the 16 KiB ceiling is message-too-large, and the 33rd message
    /// inside one rate window is rate-limited (the first 32 pass).
    #[tokio::test]
    async fn portal_oversize_and_rate_limit_refuse_with_codes() {
        let (a, b, _seen_a, _seen_b) = portal_pair().await;
        grant_both(&a, &b).await;
        let oversize = "x".repeat(id_caps::portal::MAX_PORTAL_MESSAGE_BYTES + 1);
        let error = a
            .send_portal_message("world-b".to_owned(), oversize)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::MessageTooLarge {
                bytes: id_caps::portal::MAX_PORTAL_MESSAGE_BYTES + 1,
                max: id_caps::portal::MAX_PORTAL_MESSAGE_BYTES,
            })
        );
        for index in 0..id_caps::portal::PORTAL_RATE_LIMIT {
            a.send_portal_message("world-b".to_owned(), format!("m{index}"))
                .await
                .unwrap();
        }
        let error = a
            .send_portal_message("world-b".to_owned(), "one too many".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::RateLimited {
                limit: id_caps::portal::PORTAL_RATE_LIMIT,
                window_ms: id_caps::portal::PORTAL_RATE_WINDOW_MS,
            })
        );
        let codes: Vec<String> = portal_audit(&a, "portal.send")
            .iter()
            .filter(|record| record.outcome == "refused")
            .map(|record| record.parameters["refusal"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            codes,
            [
                "portal.message_too_large".to_owned(),
                "portal.rate_limited".to_owned()
            ]
        );
    }

    /// Oracle: portals are off by default and a world with no grants
    /// and no transport installed behaves exactly as before the
    /// feature — its book is empty and a send refuses locally as
    /// sender-not-granted without touching any other world.
    #[tokio::test]
    async fn portal_unwired_world_refuses_locally_and_stays_empty() {
        let handle =
            WorldHandle::spawn(WorldCore::new("lonely-w", WorldLimits::default()).unwrap());
        assert!(
            handle
                .portal_grants()
                .await
                .unwrap()
                .peers()
                .next()
                .is_none()
        );
        let error = handle
            .send_portal_message("world-b".to_owned(), "hello?".to_owned())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            PortalSendError::Refused(id_caps::portal::PortalRefusal::SenderNotGranted {
                peer: "world-b".to_owned()
            })
        );
    }

    /// Oracle: the grant store is durable — grant and revoke entries
    /// replay from the journal into the exact book (a revoked peer
    /// replays as REVOKED, not as never-granted), and both states
    /// survive checkpoint compaction.
    #[test]
    fn portal_grants_replay_and_survive_compaction() {
        let header = || JournalEntry::Created {
            world_id: "w".to_owned(),
            version: JOURNAL_VERSION_PORTALS,
        };
        let grant = JournalEntry::PortalGrantSet {
            peer: "world-b".to_owned(),
            granted: true,
        };
        assert_eq!(journal_version_required_by(&grant), JOURNAL_VERSION_PORTALS);
        let restored =
            WorldCore::restore("w", WorldLimits::default(), [header(), grant.clone()]).unwrap();
        assert_eq!(
            restored.core.portal_grants.state("world-b"),
            Some(id_caps::portal::PortalGrantState::Granted)
        );
        let revoke = JournalEntry::PortalGrantSet {
            peer: "world-b".to_owned(),
            granted: false,
        };
        let restored =
            WorldCore::restore("w", WorldLimits::default(), [header(), grant, revoke]).unwrap();
        assert_eq!(
            restored.core.portal_grants.state("world-b"),
            Some(id_caps::portal::PortalGrantState::Revoked)
        );
        // Compaction: a core carrying a granted and a revoked peer
        // re-emits both, so a restored core sees the same book.
        let mut core = world(8);
        core.active_module_hash = Some("h".to_owned());
        core.portal_grants.grant("world-b");
        core.portal_grants.grant("world-c");
        assert!(core.portal_grants.revoke("world-c"));
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.portal_grants, core.portal_grants);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod audit_tests {
    use super::*;
    use std::fmt::Debug;
    use std::sync::Mutex;

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::{Layer, prelude::*};

    #[derive(Clone)]
    struct AuditCapture(Arc<Mutex<Vec<serde_json::Value>>>);

    impl<S: tracing::Subscriber> Layer<S> for AuditCapture {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            if event.metadata().target() != "id::audit" {
                return;
            }
            let mut visitor = AuditFieldVisitor::default();
            event.record(&mut visitor);
            let Some(json) = visitor.record else {
                return;
            };
            if let Ok(record) = serde_json::from_str(&json) {
                self.0.lock().unwrap().push(record);
            }
        }
    }

    #[derive(Default)]
    struct AuditFieldVisitor {
        record: Option<String>,
    }

    impl Visit for AuditFieldVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "audit_record" {
                self.record = Some(value.to_owned());
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
            if field.name() == "audit_record" {
                let formatted = format!("{value:?}");
                self.record = Some(serde_json::from_str::<String>(&formatted).unwrap_or(formatted));
            }
        }
    }

    struct FirstUseProgram;

    impl WorldProgram for FirstUseProgram {
        fn wants(&mut self) -> Result<Option<String>> {
            Ok(Some(
                serde_json::json!({
                    "v": 1,
                    "requests": [{"id": "first-use", "cap": "time.now"}]
                })
                .to_string(),
            ))
        }
    }

    // #1063: schema-v2 records carry a target and a unique correlation
    // id on every line, admin commands included.
    #[test]
    fn audit_records_carry_target_correlation_and_schema_v2() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new("audit-correlation", WorldLimits::default()).unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::UpdateCaps {
                    grant: vec!["time.now".to_owned()],
                    revoke: vec![],
                    reply,
                })
                .await;
            response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::UpdateCaps {
                    grant: vec![],
                    revoke: vec!["time.now".to_owned()],
                    reply,
                })
                .await;
            response.await.unwrap().unwrap();
        });

        let captured = records.lock().unwrap().clone();
        let updates: Vec<_> = captured
            .iter()
            .filter(|record| record["action"] == "UpdateCaps")
            .collect();
        assert_eq!(
            updates.len(),
            2,
            "one record per admin command: {captured:?}"
        );
        let mut correlation_ids = BTreeSet::new();
        for record in updates {
            assert_eq!(record["schema_version"], 2);
            assert_eq!(record["target"], "audit-correlation");
            let correlation_id = record["correlation_id"].as_str().unwrap();
            assert!(
                correlation_id.starts_with("audit:"),
                "correlation id shape: {correlation_id}"
            );
            assert!(
                correlation_ids.insert(correlation_id.to_owned()),
                "correlation ids are unique per record"
            );
        }
    }

    #[test]
    fn admin_audit_replays_grants_and_ceiling_and_records_refusal_once() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new(
            "audit-test",
            WorldLimits {
                grant_on_use: true,
                ..WorldLimits::default()
            },
        )
        .unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::UpdateCaps {
                    grant: vec!["time.now".to_owned()],
                    revoke: vec![],
                    reply,
                })
                .await;
            response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::SetCapsCeiling {
                    ceiling: vec!["time.now".to_owned(), "random.u64".to_owned()],
                    reply,
                })
                .await;
            response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::UpdateCaps {
                    grant: vec![],
                    revoke: vec!["time.now".to_owned()],
                    reply,
                })
                .await;
            response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::SetCapsCeiling {
                    ceiling: vec!["not-a-capability".to_owned()],
                    reply,
                })
                .await;
            assert!(response.await.unwrap().is_err());

            actor.after_change().await;
        });

        let captured = records.lock().unwrap().clone();
        let refusal_count = captured
            .iter()
            .filter(|record| record["action"] == "SetCapsCeiling" && record["outcome"] == "refused")
            .count();
        assert_eq!(refusal_count, 1, "one refused command must emit one record");
        assert_eq!(captured.len(), 5);

        let mut replayed_grants = BTreeSet::new();
        let mut replayed_ceiling = BTreeSet::new();
        for record in &captured {
            if record["outcome"] != "accepted" {
                if record["action"] == "provider_use" && record["outcome"] == "first_use_grant" {
                    replayed_grants.insert(
                        record["parameters"]["capability"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    );
                }
                continue;
            }
            match record["action"].as_str().unwrap() {
                "UpdateCaps" => {
                    for name in record["parameters"]["revoke"].as_array().unwrap() {
                        replayed_grants.remove(name.as_str().unwrap());
                    }
                    for name in record["parameters"]["grant"].as_array().unwrap() {
                        replayed_grants.insert(name.as_str().unwrap().to_owned());
                    }
                }
                "SetCapsCeiling" => {
                    replayed_ceiling = record["parameters"]["ceiling"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|name| name.as_str().unwrap().to_owned())
                        .collect();
                }
                _ => {}
            }
        }

        assert_eq!(replayed_grants, *actor.core.ledger.granted());
        assert_eq!(replayed_ceiling, actor.core.caps_ceiling);
        assert!(captured.iter().any(|record| {
            record["action"] == "provider_use"
                && record["outcome"] == "first_use_grant"
                && record["parameters"]["capability"] == "time.now"
                && record["actor"] == "program"
        }));
        assert!(captured.iter().all(|record| {
            record["schema_version"] == 2
                && record["world_id"] == "audit-test"
                && record.get("admin_token").is_none()
        }));
    }

    #[test]
    fn issuance_is_audited_alongside_revocation_without_the_token() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new("audit-test", WorldLimits::default()).unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::Issue {
                    name: "Ann".to_owned(),
                    scopes: WorldScopes::JOIN,
                    bounds: CapabilityBounds::default(),
                    reply,
                })
                .await;
            let (participant, _token) = response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::Revoke {
                    participant_id: participant.id,
                    by: None,
                    reply,
                })
                .await;
            assert!(response.await.unwrap());
        });

        let captured = records.lock().unwrap().clone();
        let issue_records: Vec<_> = captured
            .iter()
            .filter(|record| record["action"] == "Issue")
            .collect();
        assert_eq!(
            issue_records.len(),
            1,
            "one issuance must emit one audit record: {captured:?}"
        );
        let record = issue_records[0];
        assert_eq!(record["outcome"], "accepted");
        assert_eq!(record["parameters"]["name"], "Ann");
        assert!(record["parameters"]["participant_id"].is_number());
        assert!(
            record.get("token").is_none() && record["parameters"].get("token").is_none(),
            "the capability token is a secret and must never be audited"
        );
        assert!(
            captured
                .iter()
                .any(|record| record["action"] == "Revoke" && record["outcome"] == "accepted"),
            "issuance and revocation are both in the trail: {captured:?}"
        );
    }

    #[test]
    fn install_program_writability_refusal_is_audited_once() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new("audit-test", WorldLimits::default()).unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);
        actor.read_only = true;

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::InstallProgram {
                    module_hash: "deadbeef".to_owned(),
                    seed: 7,
                    provenance: None,
                    installed_by: None,
                    trust: None,
                    program: Box::new(FirstUseProgram),
                    reply,
                })
                .await;
            let error = response.await.unwrap().unwrap_err();
            assert!(
                format!("{error:#}").contains("read-only"),
                "the refusal is the writability check: {error:#}"
            );
        });

        let captured = records.lock().unwrap().clone();
        let install_records: Vec<_> = captured
            .iter()
            .filter(|record| record["action"] == "InstallProgram")
            .collect();
        assert_eq!(
            install_records.len(),
            1,
            "one writability refusal must emit exactly one audit record: {captured:?}"
        );
        assert_eq!(install_records[0]["outcome"], "refused");
        assert_eq!(install_records[0]["parameters"]["module_hash"], "deadbeef");
    }

    #[test]
    fn install_program_refuses_at_sequence_max_before_journaling() {
        #[derive(Clone)]
        struct CapturingJournal(Arc<Mutex<Vec<JournalEntry>>>);

        impl WorldJournal for CapturingJournal {
            fn append(&mut self, entry: &JournalEntry) -> Result<()> {
                self.0.lock().unwrap().push(entry.clone());
                Ok(())
            }
        }

        let persisted = Arc::new(Mutex::new(Vec::new()));
        let mut core = WorldCore::new("sequence-max", WorldLimits::default()).unwrap();
        core.sequence = u64::MAX;
        let mut actor = WorldActor::for_test(
            core,
            Box::new(FirstUseProgram),
            Some(Box::new(CapturingJournal(persisted.clone()))),
        );

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::InstallProgram {
                    module_hash: "replacement".to_owned(),
                    seed: 9,
                    provenance: None,
                    installed_by: None,
                    trust: None,
                    program: Box::new(FirstUseProgram),
                    reply,
                })
                .await;

            let error = response
                .await
                .unwrap()
                .expect_err("install must be refused");
            assert!(format!("{error:#}").contains("world sequence exhausted"));
        });

        assert_eq!(actor.core.sequence, u64::MAX);
        assert!(actor.core.events.is_empty());
        assert_eq!(actor.core.active_module_hash, None);
        assert!(persisted.lock().unwrap().is_empty());
    }

    #[test]
    fn participant_readable_queries_emit_no_admin_audit_record() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new("audit-test", WorldLimits::default()).unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor.handle(WorldCommand::Metrics { reply }).await;
            let _snapshot = response.await.unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor.handle(WorldCommand::CapsInfo { reply }).await;
            response.await.unwrap().unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::ExecutionProfileInfo { reply })
                .await;
            let _profile = response.await.unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor.handle(WorldCommand::ManifestState { reply }).await;
            let _state = response.await.unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor.handle(WorldCommand::IsolationInfo { reply }).await;
            let _isolation = response.await.unwrap();

            let (reply, response) = tokio::sync::oneshot::channel();
            actor.handle(WorldCommand::PolicyInfo { reply }).await;
            let _policy = response.await.unwrap();
        });

        let captured = records.lock().unwrap().clone();
        assert!(
            captured.is_empty(),
            "participant-readable queries must not be audited as admin: {captured:?}"
        );
    }

    #[tokio::test]
    async fn retained_audit_log_replays_to_the_final_grants() {
        // F12.6 exit test (issue #1063), retained-log half: the admin
        // actions below each produce exactly one record in the
        // world's retained audit log, and replaying that log
        // reconstructs the final grants the world reports.
        let core = WorldCore::new("audit-retained", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn(core);
        world
            .update_caps(
                vec!["chat.say".to_owned(), "time.now".to_owned()],
                Vec::new(),
            )
            .await
            .unwrap();
        world
            .update_caps(Vec::new(), vec!["time.now".to_owned()])
            .await
            .unwrap();
        // A refused ceiling change is audited and replays as nothing.
        assert!(world.set_caps_ceiling(vec!["*".to_owned()]).await.is_err());
        world
            .set_caps_ceiling(vec!["chat.say".to_owned(), "time.now".to_owned()])
            .await
            .unwrap();

        let records = world.audit_records();
        let admin_actions: Vec<(&str, &str)> = records
            .iter()
            .filter(|record| matches!(record.action.as_str(), "UpdateCaps" | "SetCapsCeiling"))
            .map(|record| (record.action.as_str(), record.outcome.as_str()))
            .collect();
        assert_eq!(
            admin_actions,
            [
                ("UpdateCaps", "accepted"),
                ("UpdateCaps", "accepted"),
                ("SetCapsCeiling", "refused"),
                ("SetCapsCeiling", "accepted"),
            ]
        );

        let report = world.caps_report().await.unwrap();
        let replayed = replay_grants(&records);
        assert_eq!(
            replayed.granted,
            report.granted.iter().cloned().collect::<BTreeSet<String>>()
        );
        assert_eq!(
            replayed.ceiling,
            Some(report.ceiling.iter().cloned().collect::<BTreeSet<String>>())
        );

        // The same records export as one OTLP log record each.
        let payload = export_otlp_json(&records);
        assert_eq!(
            payload["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
                .as_array()
                .map(Vec::len),
            Some(records.len())
        );
    }

    #[test]
    fn revocation_on_a_read_only_replica_is_audited_once_as_refused() {
        crate::test_tracing::register_audit_callsite();
        let records = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
        let _guard = tracing::subscriber::set_default(subscriber);
        crate::test_tracing::rebuild_interest_cache();

        let core = WorldCore::new("audit-test", WorldLimits::default()).unwrap();
        let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);
        actor.read_only = true;

        futures_lite::future::block_on(async {
            let (reply, response) = tokio::sync::oneshot::channel();
            actor
                .handle(WorldCommand::Revoke {
                    participant_id: 41,
                    by: None,
                    reply,
                })
                .await;
            assert!(
                !response.await.unwrap(),
                "a replica revocation reports nothing revoked"
            );
        });

        let captured = records.lock().unwrap().clone();
        let revoke_records: Vec<_> = captured
            .iter()
            .filter(|record| record["action"] == "Revoke")
            .collect();
        assert_eq!(
            revoke_records.len(),
            1,
            "one replica revocation refusal must emit exactly one audit record: {captured:?}"
        );
        assert_eq!(revoke_records[0]["outcome"], "refused");
        assert_eq!(revoke_records[0]["parameters"]["participant_id"], 41);
        assert_eq!(revoke_records[0]["parameters"]["changed"], false);
    }

    /// Deterministic command generator (xorshift64*) for the replay
    /// invariant below: a failure names its seed and replays exactly.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }

        fn subset(&mut self, caps: &[&str]) -> Vec<String> {
            caps.iter()
                .filter(|_| self.below(2) == 0)
                .map(|cap| (*cap).to_owned())
                .collect()
        }
    }

    #[test]
    fn admin_audit_replay_matches_live_state_for_generated_sequences() {
        crate::test_tracing::register_audit_callsite();
        // Oracle: the audit records themselves (model-based replay).
        // Folding the accepted records — plus first-use grants — over
        // the initial state must reproduce the live ledger grants and
        // the live ceiling after every generated sequence, and every
        // generated admin command must leave exactly one record. The
        // fixed example above samples this state-machine invariant;
        // this test hammers it with generated sequences covering
        // grants, revocations (capability and participant), ceiling
        // changes, refusals, and first-use grants (Codex P1 finding
        // on PR #4017; testing doctrine: PBT/MBT where invariants fit).
        const CAPS: [&str; 4] = ["time.now", "random.u64", "schedule.at", "chat.say"];
        for seed in 1..=16_u64 {
            let records = Arc::new(Mutex::new(Vec::new()));
            let subscriber = tracing_subscriber::registry().with(AuditCapture(records.clone()));
            let _guard = tracing::subscriber::set_default(subscriber);
            crate::test_tracing::rebuild_interest_cache();

            let core = WorldCore::new(
                "audit-test",
                WorldLimits {
                    grant_on_use: true,
                    ..WorldLimits::default()
                },
            )
            .unwrap();
            let mut actor = WorldActor::for_test(core, Box::new(FirstUseProgram), None);
            let initial_grants: BTreeSet<String> = actor.core.ledger.granted().clone();
            let initial_ceiling: BTreeSet<String> = actor.core.caps_ceiling.clone();
            let mut rng = Gen(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut issued: Vec<u64> = Vec::new();
            let mut admin_commands = 0_usize;

            futures_lite::future::block_on(async {
                for step in 0..24 {
                    match rng.below(10) {
                        0..=3 => {
                            let (reply, response) = tokio::sync::oneshot::channel();
                            actor
                                .handle(WorldCommand::UpdateCaps {
                                    grant: rng.subset(&CAPS),
                                    revoke: rng.subset(&CAPS),
                                    reply,
                                })
                                .await;
                            let _ = response.await.unwrap();
                        }
                        4..=5 => {
                            let mut ceiling = rng.subset(&CAPS);
                            if rng.below(4) == 0 {
                                ceiling.push("not-a-capability".to_owned());
                            }
                            let (reply, response) = tokio::sync::oneshot::channel();
                            actor
                                .handle(WorldCommand::SetCapsCeiling { ceiling, reply })
                                .await;
                            let _ = response.await.unwrap();
                        }
                        6 => {
                            let (reply, response) = tokio::sync::oneshot::channel();
                            actor
                                .handle(WorldCommand::Issue {
                                    name: format!("P{step}"),
                                    scopes: WorldScopes::JOIN,
                                    bounds: CapabilityBounds::default(),
                                    reply,
                                })
                                .await;
                            let (participant, _token) = response.await.unwrap().unwrap();
                            issued.push(participant.id);
                        }
                        7 => {
                            let participant_id = if issued.is_empty() || rng.below(3) == 0 {
                                999_999
                            } else {
                                issued[rng.below(issued.len() as u64) as usize]
                            };
                            let (reply, response) = tokio::sync::oneshot::channel();
                            actor
                                .handle(WorldCommand::Revoke {
                                    participant_id,
                                    by: None,
                                    reply,
                                })
                                .await;
                            let _ = response.await.unwrap();
                        }
                        _ => {
                            let (reply, response) = tokio::sync::oneshot::channel();
                            actor
                                .handle(WorldCommand::UpdateCaps {
                                    grant: vec!["not-a-capability".to_owned()],
                                    revoke: vec![],
                                    reply,
                                })
                                .await;
                            assert!(
                                response.await.unwrap().is_err(),
                                "an unknown capability must be refused"
                            );
                        }
                    }
                    admin_commands += 1;
                }
                actor.after_change().await;
            });

            let captured = records.lock().unwrap().clone();
            let admin_records = captured
                .iter()
                .filter(|record| {
                    matches!(
                        record["action"].as_str().unwrap(),
                        "UpdateCaps" | "SetCapsCeiling" | "Issue" | "Revoke"
                    )
                })
                .count();
            assert_eq!(
                admin_records, admin_commands,
                "seed {seed}: exactly one audit record per admin command"
            );

            let mut replayed_grants = initial_grants;
            let mut replayed_ceiling = initial_ceiling;
            for record in &captured {
                if record["outcome"] != "accepted" {
                    if record["action"] == "provider_use" && record["outcome"] == "first_use_grant"
                    {
                        replayed_grants.insert(
                            record["parameters"]["capability"]
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        );
                    }
                    continue;
                }
                match record["action"].as_str().unwrap() {
                    "UpdateCaps" => {
                        for name in record["parameters"]["revoke"].as_array().unwrap() {
                            replayed_grants.remove(name.as_str().unwrap());
                        }
                        for name in record["parameters"]["grant"].as_array().unwrap() {
                            replayed_grants.insert(name.as_str().unwrap().to_owned());
                        }
                    }
                    "SetCapsCeiling" => {
                        replayed_ceiling = record["parameters"]["ceiling"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|name| name.as_str().unwrap().to_owned())
                            .collect();
                    }
                    _ => {}
                }
            }
            assert_eq!(
                replayed_grants,
                *actor.core.ledger.granted(),
                "seed {seed}: replayed grants must equal the live ledger"
            );
            assert_eq!(
                replayed_ceiling, actor.core.caps_ceiling,
                "seed {seed}: replayed ceiling must equal the live ceiling"
            );
        }
    }
}

/// Structural guard for the one client projection (F11.11, #543).
///
/// Every path that hands a [`WorldEvent`] to a participant must go through
/// [`project_event_for`] (or the [`WorldHandle`] methods that wrap it), so a
/// future path cannot serialize a `WorldEvent` to a client around the
/// projection. This test pins the two serialization sites in the session and
/// the behavioural contract of the projection itself.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod projection_guard_tests {
    use crate::world::{WorldEvent, WorldEventKind, WorldScopes, project_event_for};

    /// The live stream and the catch-up page are the only two places a
    /// `WorldEvent` is serialized to a client, and both go through the
    /// projection. If a new site appears, this fails and forces a decision
    /// about whether it, too, projects.
    #[test]
    fn the_session_serializes_events_only_through_the_projection() {
        let session = include_str!("../world_session/mod.rs");
        let sites = session.matches("ServerFrame::Event {").count();
        assert_eq!(sites, 2, "exactly two client event serialization sites");
        // The live stream projects before it sends.
        assert!(
            session.contains("world.project_event(token.clone(), event.clone())"),
            "the live stream must project via WorldHandle::project_event"
        );
        // The catch-up path is `events_after`, which projects internally.
        assert!(
            session.contains("events_after"),
            "the catch-up page must come from events_after (projected)"
        );
        // `snapshot` is the third client path and is projected too.
        assert!(
            session.contains("world.snapshot(token.clone())"),
            "the snapshot must come from WorldHandle::snapshot (projected)"
        );
    }

    /// The projection's contract: a join-only viewer neither reads chat nor
    /// sees an install seed, and a program-install event keeps everything else.
    #[test]
    fn the_projection_redacts_chat_and_the_install_seed() {
        let chat = WorldEvent {
            sequence: 1,
            participant_id: 1,
            kind: WorldEventKind::Chat("hello".to_owned()),
        };
        assert_eq!(
            project_event_for(WorldScopes::JOIN, &chat),
            None,
            "a join-only viewer does not see chat"
        );
        assert_eq!(
            project_event_for(WorldScopes::CHAT, &chat),
            Some(chat.clone()),
            "a CHAT viewer sees chat"
        );

        let installed = WorldEvent {
            sequence: 2,
            participant_id: 0,
            kind: WorldEventKind::ProgramInstalled {
                module_hash: "hash".to_owned(),
                seed: 424_242,
                provenance: None,
                trust: None,
            },
        };
        let shown = project_event_for(WorldScopes::GUEST, &installed)
            .expect("a program install is visible to every participant");
        assert_eq!(shown.sequence, 2);
        match &shown.kind {
            WorldEventKind::ProgramInstalled {
                module_hash, seed, ..
            } => {
                assert_eq!(module_hash, "hash", "the module hash is kept");
                assert_eq!(*seed, 0, "the install seed is redacted");
            }
            other => panic!("unexpected kind: {other:?}"),
        }
    }
}

#[test]
fn presence_change_refuses_a_duplicated_key() {
    let text = presence_event(7, "ann", true);
    assert_eq!(presence_change(&text), Some((7, true)));
    // A lenient parse keeps the last "event" and reads a join (#8794).
    let duplicated = text.replacen('{', r#"{"event":"left","#, 1);
    assert_eq!(presence_change(&duplicated), None);
}
