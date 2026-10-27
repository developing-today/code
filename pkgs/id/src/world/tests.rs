    use super::*;

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

    fn sample_source_provenance() -> ModuleProvenance {
        ModuleProvenance::SourceBuild(SourceBuildProvenance {
            schema_version: 1,
            compiler_blake3: "compiler".to_owned(),
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
        let restored = WorldCore::restore("test-world", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.execution_profile(), profile);
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
                WorldEventKind::System { event } => self.received.push(event.clone()),
                WorldEventKind::Chat(_) | WorldEventKind::ProgramInstalled { .. } => {}
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let report = world.caps_report().await.unwrap();
        assert_eq!(report.policy, "deny");
        assert!(
            report.missing.contains(&"time.now".to_owned()),
            "{report:?}"
        );
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
            .issue(
                "ann",
                WorldScopes(WorldScopes::JOIN.bits() | WorldScopes::INPUT.bits()),
            )
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_participant_with_chat_lets_the_program_say_what_it_asks() {
        let core = WorldCore::new("caps", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let (_, token) = world
            .issue(
                "ann",
                WorldScopes(
                    WorldScopes::JOIN.bits() | WorldScopes::CHAT.bits() | WorldScopes::INPUT.bits(),
                ),
            )
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
                .is_some_and(|use_| use_.allowed > 0)
            {
                assert_eq!(report.usage["chat.say"].denied, 0);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no allowed use recorded"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest,
                scopes: WorldScopes::GUEST.0,
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
                participant: Participant {
                    id: 1,
                    display_name: "ann".to_owned(),
                },
                digest,
                scopes: WorldScopes::GUEST.0,
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
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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

    #[tokio::test]
    async fn subscribed_ticks_arrive_only_while_someone_is_present() {
        let (world, token) = caps_world_sync(false).await;
        world
            .update_caps(vec!["time.tick".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(token, b"4".to_vec()).await.unwrap();
        world.presence(1, true).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        world.presence(1, false).await;
        // Let any in-flight tick settle, then confirm the world goes quiet.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        let quiet = {
            let mut views = world.watch_views();
            views.borrow_and_update();
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
            views.has_changed().is_ok_and(|changed| !changed)
        };
        assert!(quiet, "ticks kept firing with nobody present");
        world.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn ticks_follow_the_world_grant_not_a_participant_capability() {
        let core = WorldCore::new("ticks", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        let owner_scopes = WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits());
        let (_, owner) = world.issue("ann", owner_scopes).await.unwrap();
        let short_lived = CapabilityBounds {
            expires_at: Some(unix_ms() + 150),
            uses: None,
        };
        let (_, short) = world
            .attenuate(owner.clone(), WorldScopes::GUEST, short_lived, "bo")
            .await
            .unwrap();
        world
            .update_caps(vec!["time.tick".to_owned()], Vec::new())
            .await
            .unwrap();
        world.input(owner, b"4".to_vec()).await.unwrap();
        world.presence(1, true).await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(world.chat(short, "late").await.is_err());
        let mut views = world.watch_views();
        views.borrow_and_update();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            views.has_changed().is_ok_and(|changed| changed),
            "ticks stopped when a participant's capability expired"
        );
        world.shutdown().await.unwrap();
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
        let core = WorldCore::new("tick-attribution", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(TickSay::default()));
        world
            .update_caps(
                vec!["time.tick".to_owned(), "chat.say".to_owned()],
                Vec::new(),
            )
            .await
            .unwrap();
        let (spectator, _) = world
            .issue(
                "spectator",
                WorldScopes(WorldScopes::JOIN.bits() | WorldScopes::INPUT.bits()),
            )
            .await
            .unwrap();
        world.presence(spectator.id, true).await;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let usage = world
                .caps_report()
                .await
                .unwrap()
                .usage
                .get("chat.say")
                .copied()
                .unwrap_or_default();
            if usage.allowed > 0 {
                assert_eq!(usage.denied, 0, "a tick was refused for a participant");
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no tick ever spoke");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        world.shutdown().await.unwrap();
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
    fn a_default_ceiling_is_not_written_and_old_journals_allow_the_whole_catalog() {
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
        assert_eq!(
            restored.core.ledger.allowed(),
            &crate::world_caps::catalog_names()
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
        let core = WorldCore::new("ceiling-ticks", WorldLimits::default()).unwrap();
        let world = WorldHandle::spawn_with_program(core, Box::new(Mock::new()));
        world
            .update_caps(vec!["time.tick".to_owned()], Vec::new())
            .await
            .unwrap();
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        world.presence(1, true).await;
        let mut views = world.watch_views();
        // Script 4 subscribes to `time.tick:20`, which arms it while someone is present.
        world.input(token, b"4".to_vec()).await.unwrap();
        views.borrow_and_update();
        tokio::time::timeout(std::time::Duration::from_secs(2), views.changed())
            .await
            .expect("no tick arrived while the ceiling allowed it")
            .unwrap();

        world
            .set_caps_ceiling(vec!["time.now".to_owned()])
            .await
            .unwrap();
        let report = world.caps_report().await.unwrap();
        assert!(
            report
                .usage
                .get("time.tick")
                .is_some_and(|usage| usage.denied > 0),
            "{report:?}"
        );
        assert!(!report.effective.contains(&"time.tick".to_owned()));
        // The refusal reached the program before this report; no tick follows.
        views.borrow_and_update();
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert!(
            views.has_changed().is_ok_and(|changed| !changed),
            "ticks kept firing after the ceiling dropped them"
        );

        // Lifting the ceiling admits the refused subscription again.
        let full: Vec<String> = crate::world_caps::catalog_names().into_iter().collect();
        world.set_caps_ceiling(full).await.unwrap();
        views.borrow_and_update();
        tokio::time::timeout(std::time::Duration::from_secs(2), views.changed())
            .await
            .expect("ticks did not resume once the ceiling allowed them")
            .unwrap();
        world.shutdown().await.unwrap();
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
                WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits()),
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
                WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits()),
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
        let delegator = WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits());
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
                WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits()),
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
                participant: owner,
                digest: hex_encode(&token_digest(owner_token.expose())),
                scopes: WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits())
                    .bits(),
                parent: None,
                expires_at: Some(in_an_hour()),
                uses: None,
                used: 0,
                subject: None,
            },
            JournalEntry::Issued {
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

        assert_eq!(WorldScopes::from_role("builder"), None);
        assert_eq!(
            WorldScopes::ROLES,
            &["spectator", "observer", "player", "guest", "delegator"]
        );
    }

    #[test]
    fn attenuation_only_narrows_and_needs_delegate() {
        let join = WorldScopes::JOIN;
        let delegator = WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits());
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
        let join_only = WorldScopes(WorldScopes::JOIN.bits() | WorldScopes::DELEGATE.bits());
        assert!(
            join_only.attenuate(WorldScopes::GUEST).is_none(),
            "widening"
        );
    }

    #[test]
    fn scope_names_and_effects_follow_the_scopes() {
        assert_eq!(
            WorldScopes::from_names(["join", "delegate"]),
            Some(WorldScopes(0b1001))
        );
        assert_eq!(WorldScopes::from_names(["join", "telepathy"]), None);
        assert!(WorldScopes::GUEST.allows_effect("chat.say"));
        assert!(WorldScopes::GUEST.allows_effect("chat.history"));
        let spectator = WorldScopes(WorldScopes::JOIN.bits() | WorldScopes::INPUT.bits());
        assert!(!spectator.allows_effect("chat.say"));
        assert!(!spectator.allows_effect("chat.history"));
        assert!(spectator.allows_effect("time.now"));
    }

    #[test]
    fn delegated_capabilities_are_narrow_and_die_with_their_parent() {
        let mut core = world(8);
        let root = WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits());
        let (owner, owner_token) = core
            .issue_capability("Ada", root, CapabilityBounds::default())
            .unwrap();
        let (child, child_token) = core
            .attenuate(
                &owner_token,
                WorldScopes(WorldScopes::JOIN.bits() | WorldScopes::DELEGATE.bits()),
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
        let root = WorldScopes(WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits());
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
        assert_eq!(WorldScopes::from_bits(0b10_0000), None);

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
                    WorldEventKind::System { event } => {
                        let value: serde_json::Value =
                            serde_json::from_str(event).unwrap_or(serde_json::Value::Null);
                        if value["cap"] == "result"
                            && let Some(key) = value["value"]["records"][0]["key"].as_str()
                        {
                            self.read_key = key.to_owned();
                        }
                    }
                    WorldEventKind::Chat(_) | WorldEventKind::ProgramInstalled { .. } => {}
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
}
