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
                WorldEventKind::Chat(_) | WorldEventKind::ProgramInstalled { .. } => {}
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
            self.0.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }

    /// A journaled world running a `Scheduler`, with one participant and the
    /// given grants.
    async fn scheduling_world(
        id: &str,
        grants: &[&str],
        refuse_fires: bool,
    ) -> (WorldHandle, JoinCapability, Recorded) {
        let journal = Recorded::for_world(id);
        let core = WorldCore::new(id, WorldLimits::default()).unwrap();
        let program = Scheduler {
            refuse_fires,
            ..Scheduler::default()
        };
        let world =
            WorldHandle::spawn_durable(core, Box::new(program), Some(Box::new(journal.clone())));
        let (_, token) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
        if !grants.is_empty() {
            world
                .update_caps(
                    grants.iter().map(|grant| (*grant).to_owned()).collect(),
                    Vec::new(),
                )
                .await
                .unwrap();
        }
        (world, token, journal)
    }

    /// Send one request document to the program as a participant's input.
    async fn ask(world: &WorldHandle, token: &JoinCapability, document: serde_json::Value) {
        world
            .input(token.clone(), document.to_string().into_bytes())
            .await
            .unwrap();
    }

    /// The schedule fire events the program has received so far.
    fn schedule_fires(world: &WorldHandle) -> Vec<serde_json::Value> {
        world
            .records()
            .1
            .get("fired")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// The result event the program received for request `id`, if any.
    fn schedule_result(world: &WorldHandle, id: &str) -> Option<serde_json::Value> {
        world
            .records()
            .1
            .get("results")
            .and_then(|results| results.get(id))
            .cloned()
    }

    /// Poll until `ready` holds, failing after three seconds.
    async fn eventually(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
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

        core.schedules.insert("taken".to_owned(), now + 5);
        let error = core.check_schedule("taken", now + 9, now).unwrap_err();
        assert!(error.to_string().contains("already pending"), "{error}");
        assert_eq!(
            core.schedules["taken"],
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
            full.schedules.insert(format!("s{index}"), now);
        }
        assert!(full.check_schedule("last", now, now).is_ok());
        full.schedules.insert("last".to_owned(), now);
        let error = full.check_schedule("one-more", now, now).unwrap_err();
        assert!(error.to_string().contains("pending schedules"), "{error}");
    }

    #[tokio::test]
    async fn an_ungranted_schedule_is_refused_and_counted() {
        let (world, token, journal) = scheduling_world("sched-refused", &[], false).await;
        ask(
            &world,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "nap", "at": unix_ms() + 60_000}},
            ]}),
        )
        .await;
        eventually("the refusal", || schedule_result(&world, "s1").is_some()).await;
        assert_eq!(
            schedule_result(&world, "s1").unwrap()["error"],
            "cap_denied"
        );
        let report = world.caps_report().await.unwrap();
        assert!(report.usage["schedule.at"].denied > 0, "{report:?}");
        world.shutdown().await.unwrap();
        assert!(
            journal
                .entries()
                .iter()
                .all(|entry| !matches!(entry, JournalEntry::ScheduleSet { .. })),
            "a refused request pends nothing"
        );
    }

    #[tokio::test]
    async fn a_due_schedule_fires_once_and_the_program_receives_it() {
        let (world, token, journal) = scheduling_world("sched-fire", &["schedule.at"], false).await;
        ask(
            &world,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "now", "at": unix_ms()}},
            ]}),
        )
        .await;
        eventually("the fire", || !schedule_fires(&world).is_empty()).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let fires = schedule_fires(&world);
        assert_eq!(fires.len(), 1, "{fires:?}");
        assert_eq!(fires[0]["id"], "now");
        assert!(
            fires[0].get("late").is_none(),
            "a fire in this run is not late"
        );
        world.shutdown().await.unwrap();

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

    #[tokio::test]
    async fn a_cancelled_schedule_never_fires_and_an_unknown_id_says_false() {
        let (world, token, _journal) =
            scheduling_world("sched-cancel", &["schedule.at", "schedule.cancel"], false).await;
        let at = unix_ms();
        // One pass: the cancel runs before any timer can fire the schedule.
        ask(
            &world,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "soon", "at": at}},
                {"id": "c1", "cap": "schedule.cancel", "args": {"id": "soon"}},
                {"id": "c2", "cap": "schedule.cancel", "args": {"id": "ghost"}},
            ]}),
        )
        .await;
        eventually("the cancel results", || {
            schedule_result(&world, "c2").is_some()
        })
        .await;
        assert_eq!(
            schedule_result(&world, "s1").unwrap()["value"]["fires_at"],
            at
        );
        assert_eq!(
            schedule_result(&world, "c1").unwrap()["value"]["cancelled"],
            true
        );
        assert_eq!(
            schedule_result(&world, "c2").unwrap()["value"]["cancelled"],
            false
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            schedule_fires(&world).is_empty(),
            "a cancelled schedule must not fire"
        );
        world.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn refused_schedule_requests_pend_nothing() {
        let (world, token, journal) =
            scheduling_world("sched-bad", &["schedule.at", "schedule.cancel"], false).await;
        let later = unix_ms() + 3_600_000;
        let far = unix_ms() + crate::world_caps::MAX_SCHEDULE_HORIZON_MS + 60_000;
        let long_id = "x".repeat(65);
        ask(
            &world,
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
        eventually("the last result", || {
            schedule_result(&world, "bad-cancel").is_some()
        })
        .await;
        assert_eq!(schedule_result(&world, "ok").unwrap()["ok"], true);
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
                schedule_result(&world, id).unwrap()["error"],
                "bad_args",
                "{id}"
            );
        }
        world.shutdown().await.unwrap();
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
        let (world, token, journal) =
            scheduling_world("sched-refuse", &["schedule.at"], true).await;
        ask(
            &world,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "stuck", "at": unix_ms()}},
            ]}),
        )
        .await;
        eventually("the request result", || {
            schedule_result(&world, "s1").is_some()
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // The program is unavailable now, but the actor still answers.
        assert!(world.caps_report().await.is_ok());
        world.shutdown().await.unwrap();

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
                at_ms: unix_ms() - 5_000,
            },
        ];
        let restored = WorldCore::restore("sched-late", WorldLimits::default(), entries).unwrap();
        let journal = Recorded::default();
        let world = WorldHandle::spawn_durable(
            restored.core,
            Box::new(Scheduler::default()),
            Some(Box::new(journal.clone())),
        );
        eventually("the late fire", || !schedule_fires(&world).is_empty()).await;
        let fires = schedule_fires(&world);
        assert_eq!(fires.len(), 1, "{fires:?}");
        assert_eq!(fires[0]["id"], "overdue");
        assert_eq!(fires[0]["late"], true);
        world.shutdown().await.unwrap();
        assert!(
            journal.entries().iter().any(
                |entry| matches!(entry, JournalEntry::ScheduleFired { id } if id == "overdue")
            )
        );
    }

    #[tokio::test]
    async fn a_pending_schedule_survives_a_restart_and_fires_on_time() {
        let at_ms = unix_ms() + 700;
        let entries = vec![
            JournalEntry::Created {
                world_id: "sched-later".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::ScheduleSet {
                id: "later".to_owned(),
                at_ms,
            },
        ];
        let restored = WorldCore::restore("sched-later", WorldLimits::default(), entries).unwrap();
        assert_eq!(restored.core.schedules.get("later"), Some(&at_ms));
        let world = WorldHandle::spawn_durable(restored.core, Box::new(Scheduler::default()), None);
        assert!(schedule_fires(&world).is_empty(), "not due yet");
        eventually("the scheduled fire", || !schedule_fires(&world).is_empty()).await;
        let fires = schedule_fires(&world);
        assert_eq!(fires[0]["id"], "later");
        assert!(fires[0].get("late").is_none(), "it was not overdue at open");
        world.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn replay_reproduces_the_pending_and_fired_sets() {
        let (world, token, journal) =
            scheduling_world("sched-replay", &["schedule.at", "schedule.cancel"], false).await;
        ask(
            &world,
            &token,
            serde_json::json!({"requests": [
                {"id": "s1", "cap": "schedule.at", "args": {"id": "keep", "at": unix_ms() + 3_600_000}},
                {"id": "s2", "cap": "schedule.at", "args": {"id": "now", "at": unix_ms()}},
                {"id": "s3", "cap": "schedule.at", "args": {"id": "gone", "at": unix_ms() + 3_600_000}},
                {"id": "c3", "cap": "schedule.cancel", "args": {"id": "gone"}},
            ]}),
        )
        .await;
        eventually("the fire and the cancel", || {
            !schedule_fires(&world).is_empty() && schedule_result(&world, "c3").is_some()
        })
        .await;
        let live_fired: Vec<String> = schedule_fires(&world)
            .iter()
            .filter_map(|fire| fire["id"].as_str().map(str::to_owned))
            .collect();
        world.shutdown().await.unwrap();

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
        core.schedules.insert("a".to_owned(), now + 100);
        core.schedules.insert("b".to_owned(), now + 200);
        let entries = core.checkpoint_entries("state".to_owned()).unwrap();
        for (id, at_ms) in [("a", now + 100), ("b", now + 200)] {
            let set = JournalEntry::ScheduleSet {
                id: id.to_owned(),
                at_ms,
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
        };
        let checkpoint = |schedules: Option<Vec<(String, u64)>>| JournalEntry::Checkpoint {
            sequence: 0,
            module_hash: "h".to_owned(),
            seed: 1,
            provenance: None,
            execution_profile: None,
            caps_ceiling: None,
            schedules,
            snapshot: "s".to_owned(),
            events: Vec::new(),
        };
        let agreed = [header(), set(), checkpoint(Some(vec![("a".to_owned(), 5)]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), agreed).is_ok());
        let omitted = [header(), set(), checkpoint(None)];
        assert!(WorldCore::restore("w", WorldLimits::default(), omitted).is_err());
        let unwritten = [header(), checkpoint(Some(vec![("a".to_owned(), 5)]))];
        assert!(WorldCore::restore("w", WorldLimits::default(), unwritten).is_err());
        let duplicated = [
            header(),
            set(),
            checkpoint(Some(vec![("a".to_owned(), 5), ("a".to_owned(), 5)])),
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
            },
            JournalEntry::ScheduleSet {
                id: "x".to_owned(),
                at_ms: 2,
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), twice).is_err());
        let malformed = [
            header(),
            JournalEntry::ScheduleSet {
                id: "a\nb".to_owned(),
                at_ms: 1,
            },
        ];
        assert!(WorldCore::restore("w", WorldLimits::default(), malformed).is_err());
    }

    #[test]
    fn schedules_wake_the_loop_only_while_the_program_can_take_them() {
        let mut schedules = BTreeMap::new();
        schedules.insert("a".to_owned(), 2_000u64);
        assert!(next_schedule_wake(&schedules, 1_000, true).is_some());
        assert!(
            next_schedule_wake(&schedules, 3_000, true).is_some(),
            "an overdue schedule wakes the loop now"
        );
        assert!(
            next_schedule_wake(&schedules, 3_000, false).is_none(),
            "a program that refuses fires never wakes the loop for one"
        );
        assert!(next_schedule_wake(&BTreeMap::new(), 1_000, true).is_none());
    }

    #[test]
    fn schedule_entries_keep_their_journal_tags() {
        let set = serde_json::to_value(JournalEntry::ScheduleSet {
            id: "a".to_owned(),
            at_ms: 7,
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
