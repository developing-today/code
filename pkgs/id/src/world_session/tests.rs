#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use tokio::sync::mpsc;

    use ed25519_dalek::{Signer, SigningKey};

    use super::*;
    use crate::directory::{Actor, Level, Member};
    use crate::directory_auth::sign_request;
    use crate::world::{
        Participant, WorldCore, WorldEvent, WorldHandle, WorldLimits, WorldScopes, hex_encode,
        unix_ms,
    };
    use crate::world_hub::WorldHub;
    use crate::world_session::*;
    use std::time::Duration;

    /// In-memory transport: tests push frames in and read frames out.
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

    struct Peer {
        to_server: mpsc::Sender<Inbound>,
        from_server: mpsc::Receiver<String>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Peer {
        fn connect(service: WorldService, config: SessionConfig) -> Self {
            Self::connect_hub(WorldHub::single(service), config)
        }

        fn connect_hub(hub: WorldHub, config: SessionConfig) -> Self {
            let (to_server, inbound) = mpsc::channel(16);
            let (outbound, from_server) = mpsc::channel(64);
            let task = tokio::spawn(async move {
                let mut io = ChannelIo { inbound, outbound };
                run_session_with(&hub, &mut io, config).await;
            });
            Self {
                to_server,
                from_server,
                task,
            }
        }

        async fn say(&self, frame: serde_json::Value) {
            self.to_server
                .send(Inbound::Text(frame.to_string()))
                .await
                .unwrap();
        }

        /// Sends the frame text as given, so a test controls field order.
        async fn say_text(&self, text: &str) {
            self.to_server
                .send(Inbound::Text(text.to_owned()))
                .await
                .unwrap();
        }

        async fn next_text(&mut self) -> Option<String> {
            // An upload takes several seconds in a debug build, so 5 s was too
            // tight. The deadline still bounds a real hang.
            tokio::time::timeout(Duration::from_mins(1), self.from_server.recv())
                .await
                .expect("timed out waiting for a server frame")
        }

        async fn next(&mut self) -> Option<serde_json::Value> {
            self.next_text()
                .await
                .map(|frame| serde_json::from_str(&frame).unwrap())
        }
    }

    /// A program that does nothing, for tests that only install one.
    struct NoProgram;

    impl crate::world::WorldProgram for NoProgram {}

    /// Publishes 1024 records of about 1 KiB, so one query needs several
    /// pages (the page budget is 768 KiB).
    struct Bulky;

    impl crate::world::WorldProgram for Bulky {
        fn records(&mut self) -> anyhow::Result<Option<String>> {
            let payload = "x".repeat(1024);
            let records: crate::world::Records = (0..1024)
                .map(|i| {
                    (
                        format!("key-{i:04}"),
                        serde_json::Value::from(payload.clone()),
                    )
                })
                .collect();
            Ok(Some(serde_json::to_string(&records)?))
        }
    }

    fn service_with_records(admin: Option<&str>) -> WorldService {
        WorldService::new(
            WorldHandle::spawn_with_program(
                WorldCore::new("lobby", WorldLimits::default()).unwrap(),
                Box::new(Bulky),
            ),
            admin.map(str::to_owned),
        )
    }

    fn service(admin: Option<&str>) -> WorldService {
        WorldService::new(
            WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
            admin.map(str::to_owned),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn committed_event_burst_coalesces_views_but_keeps_event_frames_immediate() {
        // Match the 383-byte roc-grid 80x24 guest frame in the F11.9 baseline
        // table; this test compares the full JSON frames sent on the wire.
        const F11_9_ROC_GRID_FRAME_BYTES: usize = 383;
        struct ViewProgram;
        impl crate::world::WorldProgram for ViewProgram {
            fn view(&mut self, _viewer: &Participant) -> anyhow::Result<Option<String>> {
                Ok(Some("x".repeat(F11_9_ROC_GRID_FRAME_BYTES)))
            }
        }

        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(ViewProgram),
        );
        let service = WorldService::new(world.clone(), None);
        let (_, viewer) = world.issue("viewer", WorldScopes::GUEST).await.unwrap();
        let (_, writer) = world.issue("writer", WorldScopes::GUEST).await.unwrap();
        let mut peer = Peer::connect(service, SessionConfig::default());
        peer.say(serde_json::json!({"type":"join","capability":viewer.expose()}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "snapshot");
        assert_eq!(peer.next().await.unwrap()["type"], "view");

        for text in ["one", "two", "three"] {
            world.chat(writer.clone(), text).await.unwrap();
        }
        let mut latest_sequence = 0;
        let mut event_frame_bytes = 0;
        for _ in 0..3 {
            let frame = peer.next_text().await.unwrap();
            event_frame_bytes += frame.len();
            let event: serde_json::Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(event["type"], "event");
            latest_sequence = event["event"]["sequence"].as_u64().unwrap();
        }
        assert!(
            peer.from_server.try_recv().is_err(),
            "no view before the window"
        );

        tokio::time::advance(VIEW_COALESCE_WINDOW).await;
        let view_frame = peer.next_text().await.unwrap();
        let view_frame_bytes = view_frame.len();
        let view: serde_json::Value = serde_json::from_str(&view_frame).unwrap();
        assert_eq!(view["type"], "view");
        assert_eq!(
            view["data"].as_str().unwrap().len(),
            F11_9_ROC_GRID_FRAME_BYTES
        );
        assert_eq!(view["sequence"].as_u64(), Some(latest_sequence));
        let batched_bytes = event_frame_bytes + view_frame_bytes;
        let unbatched_bytes = event_frame_bytes + 3 * view_frame_bytes;
        assert_eq!(unbatched_bytes - batched_bytes, 2 * view_frame_bytes);

        world.chat(writer, "after window").await.unwrap();
        let event = peer.next().await.unwrap();
        assert_eq!(event["type"], "event");
        let latest_sequence = event["event"]["sequence"].as_u64().unwrap();
        assert!(peer.from_server.try_recv().is_err());
        tokio::time::advance(VIEW_COALESCE_WINDOW).await;
        let view = peer.next().await.unwrap();
        assert_eq!(view["type"], "view");
        assert_eq!(view["sequence"].as_u64(), Some(latest_sequence));
    }

    #[tokio::test]
    async fn execution_profile_change_is_admin_only_and_does_not_replace_live_program() {
        struct MarkerProgram;
        impl crate::world::WorldProgram for MarkerProgram {
            fn view(&mut self, _viewer: &Participant) -> anyhow::Result<Option<String>> {
                Ok(Some("existing instance".to_owned()))
            }
        }

        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(MarkerProgram),
        );
        let service = WorldService::new(world.clone(), Some("admin".to_owned()));
        let requested = crate::world_limits::WorldExecutionProfile {
            wasm_fuel: Some(1000),
            ..Default::default()
        };
        assert!(
            service
                .set_execution_profile(Some("wrong"), requested)
                .await
                .is_err()
        );
        assert_eq!(
            world.execution_profile().await.unwrap(),
            crate::world_limits::WorldExecutionProfile::default()
        );

        let update = service
            .set_execution_profile(Some("admin"), requested)
            .await
            .unwrap();
        assert_eq!(update.requested, requested);
        assert!(update.applies_on_next_program_install_or_process_restart);
        let (_, token) = world.issue("guest", WorldScopes::GUEST).await.unwrap();
        assert_eq!(
            world.view(token).await.unwrap().as_deref(),
            Some("existing instance"),
            "updating the profile must not replace or mutate the running program"
        );
    }

    #[tokio::test]
    async fn execution_profile_protocol_returns_the_deferred_application_notice() {
        let service = service(Some("admin"));
        let mut peer = Peer::connect(service, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_execution_profile",
            "admin_token": "admin",
            "profile": {"wasm_fuel": 1234},
        }))
        .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "execution_profile");
        assert_eq!(response["update"]["requested"]["wasm_fuel"], 1234);
        assert_eq!(
            response["update"]["applies_on_next_program_install_or_process_restart"],
            true
        );
        assert!(
            response["message"]
                .as_str()
                .unwrap()
                .contains("not to the running instance")
        );
    }

    #[tokio::test]
    async fn metrics_frame_reports_each_host_metric_after_a_scripted_session() {
        // F11.7 slice 1: after a scripted session (invite, join, input),
        // the `metrics` frame reads back every host metric `id world
        // stats` surfaces.
        let svc = service(Some("admin"));
        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({
            "type": "invite",
            "admin_token": "admin",
            "display_name": "player",
            "role": "player",
        }))
        .await;
        let capability = host.next().await.unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();

        // A capability that does not join is refused, like `info`.
        let mut stranger = Peer::connect(svc.clone(), SessionConfig::default());
        stranger
            .say(serde_json::json!({"type":"metrics","capability":"bogus"}))
            .await;
        assert_eq!(stranger.next().await.unwrap()["type"], "error");

        let mut player = Peer::connect(svc.clone(), SessionConfig::default());
        player
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(player.next().await.unwrap()["type"], "snapshot");
        player
            .say(serde_json::json!({"type":"input","data_hex":"6c"}))
            .await;
        let _ = player.next().await;

        let mut reader = Peer::connect(svc.clone(), SessionConfig::default());
        reader
            .say(serde_json::json!({"type":"metrics","capability":capability}))
            .await;
        let reply = reader.next().await.unwrap();
        assert_eq!(reply["type"], "metrics", "{reply}");
        let metrics = &reply["metrics"];
        assert!(
            metrics["commands_total"].as_u64().unwrap() > 0,
            "actor commands were handled: {metrics}"
        );
        assert_eq!(
            metrics["command_latency"]["count"], metrics["commands_total"],
            "every handled command lands in the latency histogram: {metrics}"
        );
        // This world is in-memory: no journal, so appends and checkpoints
        // read as a present, empty metric rather than a missing one.
        assert_eq!(metrics["journal_appends_total"], 0, "{metrics}");
        assert_eq!(metrics["journal_append_latency"]["count"], 0, "{metrics}");
        assert_eq!(metrics["checkpoints_total"], 0, "{metrics}");
        assert_eq!(metrics["checkpoint_duration"]["count"], 0, "{metrics}");
        assert!(
            metrics["frame_bytes_total"].as_u64().unwrap() > 0,
            "session frames were metered: {metrics}"
        );
        assert_eq!(
            metrics["sessions"], 1,
            "the joined player is one live session: {metrics}"
        );
        assert!(metrics["actor_queue_depth"].is_u64(), "{metrics}");
        assert!(metrics["actor_queue_depth_max"].is_u64(), "{metrics}");
    }

    /// Read frames until a non-snapshot/non-view reply arrives.
    async fn next_reply(peer: &mut Peer) -> serde_json::Value {
        loop {
            let frame = peer.next().await.unwrap();
            if frame["type"] != "snapshot" && frame["type"] != "view" {
                return frame;
            }
        }
    }

    #[tokio::test]
    async fn joined_metrics_frame_is_refused_after_revocation() {
        // #1506: the joined-phase metrics handler must capability-check
        // like the query phase — a revoked participant keeps no metrics
        // access even if the frame beats the revocation broadcast.
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let svc = WorldService::new(world.clone(), Some("admin".to_owned()));
        let (participant, token) = world.issue("player", WorldScopes::GUEST).await.unwrap();
        let cap = token.expose().to_owned();

        let mut player = Peer::connect(svc.clone(), SessionConfig::default());
        player
            .say(serde_json::json!({"type":"join","capability":cap}))
            .await;
        assert_eq!(player.next().await.unwrap()["type"], "snapshot");

        // While the capability is live, a joined metrics frame is served.
        player
            .say(serde_json::json!({"type":"metrics","capability":cap}))
            .await;
        assert_eq!(next_reply(&mut player).await["type"], "metrics");

        // Queue the frame, then revoke: whichever the session loop
        // processes first — the frame (gate refuses: "join denied") or
        // the revocation broadcast (session closes) — the reply is an
        // error, never metrics.
        player
            .say(serde_json::json!({"type":"metrics","capability":cap}))
            .await;
        assert!(world.revoke(participant.id).await.unwrap());
        let reply = next_reply(&mut player).await;
        assert_eq!(
            reply["type"], "error",
            "revoked metrics must be refused: {reply}"
        );
    }

    #[tokio::test]
    async fn joined_caps_frame_is_refused_after_revocation() {
        // #1506 (same pass): the joined-phase caps handler has the same
        // token-less shape and gets the same participant gate.
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let svc = WorldService::new(world.clone(), Some("admin".to_owned()));
        let (participant, token) = world.issue("player", WorldScopes::GUEST).await.unwrap();
        let cap = token.expose().to_owned();

        let mut player = Peer::connect(svc.clone(), SessionConfig::default());
        player
            .say(serde_json::json!({"type":"join","capability":cap}))
            .await;
        assert_eq!(player.next().await.unwrap()["type"], "snapshot");

        player
            .say(serde_json::json!({"type":"caps","capability":cap}))
            .await;
        assert_eq!(next_reply(&mut player).await["type"], "caps");

        // Queued-frame-then-revoke, as in the metrics test above.
        player
            .say(serde_json::json!({"type":"caps","capability":cap}))
            .await;
        assert!(world.revoke(participant.id).await.unwrap());
        let reply = next_reply(&mut player).await;
        assert_eq!(
            reply["type"], "error",
            "revoked caps must be refused: {reply}"
        );
    }

    #[tokio::test]
    async fn caps_ceiling_is_admin_only_and_applies_at_once() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world.clone(), Some("admin".to_owned()));
        let full = world.caps_report().await.unwrap().ceiling;
        assert!(
            service
                .set_caps_ceiling(Some("wrong"), vec!["time.now".to_owned()])
                .await
                .is_err()
        );
        assert!(
            service
                .set_caps_ceiling(None, vec!["time.now".to_owned()])
                .await
                .is_err()
        );
        assert_eq!(
            world.caps_report().await.unwrap().ceiling,
            full,
            "a refused change changes nothing"
        );

        let report = service
            .set_caps_ceiling(Some("admin"), vec!["time.now".to_owned()])
            .await
            .unwrap();
        assert_eq!(report.ceiling, vec!["time.now".to_owned()]);
        assert_eq!(
            world.caps_report().await.unwrap().ceiling,
            vec!["time.now".to_owned()]
        );
    }

    #[tokio::test]
    async fn caps_ceiling_protocol_round_trips_and_says_it_applies_now() {
        let mut peer = Peer::connect(service(Some("admin")), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_caps_ceiling",
            "admin_token": "admin",
            "ceiling": ["time.now"],
        }))
        .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "caps_ceiling");
        assert_eq!(
            response["report"]["ceiling"],
            serde_json::json!(["time.now"])
        );
        assert!(
            response["message"]
                .as_str()
                .unwrap()
                .contains("applies immediately")
        );

        let mut peer = Peer::connect(service(Some("admin")), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_caps_ceiling",
            "admin_token": "wrong",
            "ceiling": [],
        }))
        .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");

        let mut peer = Peer::connect(service(Some("admin")), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_caps_ceiling",
            "admin_token": "admin",
            "ceiling": ["*"],
        }))
        .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");
    }

    #[tokio::test]
    async fn manifest_schedules_are_admin_only_and_replace() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world.clone(), Some("admin".to_owned()));
        let at = unix_ms() + 60_000;
        assert!(
            service
                .set_schedule(Some("wrong"), "nap".to_owned(), at)
                .await
                .is_err()
        );
        assert!(
            service
                .set_schedule(None, "nap".to_owned(), at)
                .await
                .is_err()
        );
        assert!(
            world.manifest_state().await.unwrap().1.is_empty(),
            "a refused change changes nothing"
        );

        let schedules = service
            .set_schedule(Some("admin"), "nap".to_owned(), at)
            .await
            .unwrap();
        assert_eq!(
            schedules,
            std::collections::BTreeMap::from([("nap".to_owned(), at)])
        );
        // Re-setting the same id replaces its fire time: that is how a
        // manifest reconciles a schedule whose time drifted.
        let later = at + 60_000;
        let schedules = service
            .set_schedule(Some("admin"), "nap".to_owned(), later)
            .await
            .unwrap();
        assert_eq!(
            schedules,
            std::collections::BTreeMap::from([("nap".to_owned(), later)])
        );
        let (seed, schedules) = world.manifest_state().await.unwrap();
        assert_eq!(seed, 0, "no install yet, so no install seed");
        assert_eq!(schedules.get("nap"), Some(&later));

        let schedules = service
            .cancel_schedule(Some("admin"), "nap".to_owned())
            .await
            .unwrap();
        assert!(schedules.is_empty());
        // Cancelling an absent id is a no-op, not an error.
        assert!(
            service
                .cancel_schedule(Some("admin"), "nap".to_owned())
                .await
                .unwrap()
                .is_empty()
        );
    }

    /// #807: a module saved only in the module directory (the native
    /// install path never pins blobs) is still retrievable by hash for an
    /// admin reconciling a manifest, and an unknown hash is refused.
    #[tokio::test]
    async fn stored_module_bytes_fall_back_to_the_module_directory() {
        let dir =
            std::env::temp_dir().join(format!("id-stored-module-test-{}", std::process::id()));
        let modules = crate::world_store::ModuleDir::new(dir.clone());
        let bytes: &[u8] = b"native-module-bytes";
        let hash = modules.save(bytes).unwrap();

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, Some("admin".to_owned())).with_module_dir(modules);
        assert_eq!(service.stored_module_bytes(&hash).await.unwrap(), bytes);
        assert!(service.stored_module_bytes(&"0".repeat(64)).await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn schedule_and_world_state_frames_round_trip_and_hide_the_seed() {
        let svc = service(Some("admin"));
        let world = svc.world().clone();
        let (_, guest) = world.issue("ada", WorldScopes::GUEST).await.unwrap();
        let cap = guest.expose().to_owned();
        world
            .install_program("ab".repeat(32), 4242, Box::new(NoProgram))
            .await
            .unwrap();
        let at = unix_ms() + 60_000;

        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_schedule",
            "admin_token": "admin",
            "id": "nap",
            "at_ms": at,
        }))
        .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "schedules");
        assert_eq!(response["schedules"], serde_json::json!({"nap": at}));

        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "set_schedule",
            "admin_token": "wrong",
            "id": "nap",
            "at_ms": at,
        }))
        .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");

        // A capability reader sees the schedules but never the seed (#240).
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type": "world_state", "capability": cap}))
            .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "world_state");
        assert_eq!(response["seed"], serde_json::Value::Null);
        assert_eq!(response["schedules"], serde_json::json!({"nap": at}));

        // The admin reader sees the real install seed.
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type": "world_state", "admin_token": "admin"}))
            .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "world_state");
        assert_eq!(response["seed"], 4242);
        assert_eq!(response["schedules"], serde_json::json!({"nap": at}));

        // No credential at all is refused.
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type": "world_state"})).await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");

        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "cancel_schedule",
            "admin_token": "admin",
            "id": "nap",
        }))
        .await;
        let response = peer.next().await.unwrap();
        assert_eq!(response["type"], "schedules");
        assert_eq!(response["schedules"], serde_json::json!({}));
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn source_compile_sees_the_effective_list_not_the_raw_grants() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("platform.roc"), "x").unwrap();
        std::fs::create_dir_all(root.path().join("targets/wasm32")).unwrap();
        std::fs::write(root.path().join("targets/wasm32/host.wasm"), b"\0asm").unwrap();
        std::fs::write(root.path().join("Key.roc"), "module []\n").unwrap();
        std::fs::write(root.path().join("Screen.roc"), "module []\n").unwrap();
        let compiler =
            Compiler::new(root.path().to_path_buf(), "/nonexistent/roc".to_owned()).unwrap();
        let svc = service(Some("admin")).with_compiler(compiler);
        svc.world()
            .update_caps(vec!["*".to_owned()], Vec::new())
            .await
            .unwrap();
        let spec = CompileSpec {
            files: vec![(
                "main.roc".to_owned(),
                "import pf.Screen\napp [x] {}".to_owned(),
            )],
            seed: 1,
            native: false,
        };

        svc.set_caps_ceiling(Some("admin"), vec!["Key".to_owned()])
            .await
            .unwrap();
        let error = svc
            .compile_and_install(Some("admin"), &spec)
            .await
            .unwrap_err();
        assert!(error.contains("main.roc imports Screen"), "{error}");

        // Inside the ceiling the same import passes the gate and reaches the
        // compiler, which is not installed here.
        svc.set_caps_ceiling(Some("admin"), vec!["Key".to_owned(), "Screen".to_owned()])
            .await
            .unwrap();
        let error = svc
            .compile_and_install(Some("admin"), &spec)
            .await
            .unwrap_err();
        assert!(!error.contains("imports Screen"), "{error}");
    }

    struct FixedScreen;

    impl crate::world::WorldProgram for FixedScreen {
        fn view(&mut self, _viewer: &Participant) -> anyhow::Result<Option<String>> {
            Ok(Some(
                r#"{"kind":"screen","version":1,"width":4,"height":2,"rows":["界ab","wxyz"]}"#
                    .to_owned(),
            ))
        }
    }

    #[tokio::test]
    async fn viewport_is_session_local_and_never_advances_the_world_sequence() {
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(FixedScreen),
        );
        let svc = WorldService::new(world.clone(), None);
        let scopes = WorldScopes::from_names(["join", "input"]).unwrap();
        let (_, capability) = world.issue("player".to_owned(), scopes).await.unwrap();
        let mut peer = Peer::connect(svc, SessionConfig::default());
        peer.say(serde_json::json!({"type":"join","capability":capability.expose()}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "snapshot");
        assert_eq!(peer.next().await.unwrap()["type"], "view");
        let sequence = world
            .snapshot(capability.clone())
            .await
            .unwrap()
            .current_sequence;

        peer.say(serde_json::json!({"type":"resize","width":1,"height":1}))
            .await;
        let view = peer.next().await.unwrap();
        assert_eq!(view["type"], "view");
        assert_eq!(view["surface"]["width"], 1);
        assert_eq!(view["surface"]["height"], 1);
        assert_eq!(view["surface"]["cells"][0]["symbol"], " ");
        assert_eq!(
            world.snapshot(capability).await.unwrap().current_sequence,
            sequence
        );
    }

    /// A server with Ann and Bo as friends and no outbox. When `bo_home_is_remote`,
    /// Bo's artifacts are held on another server, so removing Bo must be sent.
    async fn friends_without_outbox(
        bo_home_is_remote: bool,
    ) -> (WorldService, crate::directory_auth::Caller, String) {
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        let (ann_secret, _) = core.directory_mut().sign_up("Ann").unwrap();
        let (bo_secret, _) = core.directory_mut().sign_up("Bo").unwrap();
        let ann_id = core.directory().account_for(&ann_secret).unwrap();
        let bo_id = core.directory().account_for(&bo_secret).unwrap();
        if bo_home_is_remote {
            let home = crate::artifact::HomeServer {
                endpoint: iroh::SecretKey::from_bytes(&[5; 32]).public().to_string(),
                world_id: "home".to_owned(),
            };
            core.directory_mut()
                .declare_home(&bo_secret, vec![home])
                .unwrap();
        }
        let svc = WorldService::new(WorldHandle::spawn(core), None);
        let ann = crate::directory_auth::Caller::from_secret(&ann_secret);
        let bo = crate::directory_auth::Caller::from_secret(&bo_secret);
        svc.directory(
            ann.clone(),
            crate::directory_view::DirectoryAction::RequestFriend {
                to: bo_id.clone(),
                remote: None,
            },
        )
        .await
        .unwrap();
        svc.directory(
            bo,
            crate::directory_view::DirectoryAction::AcceptFriend {
                from: ann_id,
                remote: None,
            },
        )
        .await
        .unwrap();
        (svc, ann, bo_id)
    }

    async fn friends_of_ann(
        svc: &WorldService,
        ann: &crate::directory_auth::Caller,
    ) -> Option<Vec<String>> {
        let view = svc
            .directory(ann.clone(), crate::directory_view::DirectoryAction::View)
            .await
            .unwrap()
            .view;
        view.accounts
            .iter()
            .find(|a| a.name == "Ann")
            .unwrap()
            .friends
            .clone()
    }

    #[tokio::test]
    async fn a_local_friend_removal_needs_no_outbox() {
        let (svc, ann, bo) = friends_without_outbox(false).await;
        svc.directory(
            ann.clone(),
            crate::directory_view::DirectoryAction::RemoveFriend { other: bo },
        )
        .await
        .unwrap();
        assert_eq!(friends_of_ann(&svc, &ann).await, Some(Vec::new()));
    }

    #[tokio::test]
    async fn a_removal_of_a_friend_on_another_server_is_refused_without_an_outbox() {
        let (svc, ann, bo) = friends_without_outbox(true).await;
        let refused = svc
            .directory(
                ann.clone(),
                crate::directory_view::DirectoryAction::RemoveFriend { other: bo.clone() },
            )
            .await
            .unwrap_err();
        assert!(
            format!("{refused:#}").contains("cannot send to other servers"),
            "{refused:#}"
        );
        assert_eq!(friends_of_ann(&svc, &ann).await, Some(vec![bo]));
    }

    #[tokio::test]
    async fn a_signed_moderator_sets_isolation_and_a_forged_request_does_not() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let moderator = hex_encode(&key.verifying_key().to_bytes());
        let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
        core.directory_mut()
            .sign_up_keyed(&moderator, "Mo")
            .unwrap();
        let (credential, _) = core.directory_mut().sign_up("Founder").unwrap();
        let founder = core.directory().account_for(&credential).unwrap();
        let (group, _) = core
            .directory_mut()
            .create_group(&founder, "staff", "", WorldScopes::GUEST)
            .unwrap();
        core.directory_mut()
            .set_member(
                Actor::Server,
                group,
                Member::Account {
                    id: moderator.clone(),
                },
                Some(Level::Moderator),
            )
            .unwrap();
        let hub = WorldHub::single(WorldService::new(WorldHandle::spawn(core), None));
        let lease = hub.lease(None, None).await.unwrap();
        let target = format!("/world/{}/isolation", lease.name());
        let now = unix_ms();

        let mut forged = sign_request(&key, "PUT", &target, b"unisolated", now, "forged");
        let last = forged.signature.pop().unwrap();
        forged.signature.push(if last == '0' { '1' } else { '0' });
        let other_body = sign_request(&key, "PUT", &target, b"isolated", now, "other-body");
        for signed in [&forged, &other_body] {
            assert!(
                set_isolation_frame(&hub, &lease, Isolation::Unisolated, None, Some(signed))
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            lease.service().world().isolation().await.unwrap(),
            Isolation::Isolated
        );

        let signed = sign_request(&key, "PUT", &target, b"unisolated", now, "good");
        assert_eq!(
            set_isolation_frame(&hub, &lease, Isolation::Unisolated, None, Some(&signed))
                .await
                .unwrap(),
            Isolation::Unisolated
        );
    }

    #[tokio::test]
    async fn a_bad_admin_token_is_refused_for_force_and_override() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        world
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        let hub = WorldHub::single(WorldService::new(world, Some("secret".to_owned())));
        let lease = hub.lease(None, None).await.unwrap();
        let force_guess = ClientFrame::ForceCrossWorldWrite {
            admin_token: "guess".to_owned(),
        };
        let override_guess = ClientFrame::SetFileScope {
            scope: FileScope::Unrestricted,
            write_override: Some(true),
            admin_token: Some("guess".to_owned()),
            signed: None,
        };
        assert!(
            policy_frame(&hub, &lease, &force_guess)
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            policy_frame(&hub, &lease, &override_guess)
                .await
                .unwrap()
                .is_err()
        );
        let policy = lease.service().world().policy().await.unwrap();
        assert!(!policy.cross_world_forced && !policy.file_write_override);

        let force = ClientFrame::ForceCrossWorldWrite {
            admin_token: "secret".to_owned(),
        };
        let forced = policy_frame(&hub, &lease, &force).await.unwrap().unwrap();
        assert!(forced.cross_world_forced);
        let override_on = ClientFrame::SetFileScope {
            scope: FileScope::Unrestricted,
            write_override: Some(true),
            admin_token: Some("secret".to_owned()),
            signed: None,
        };
        let set = policy_frame(&hub, &lease, &override_on)
            .await
            .unwrap()
            .unwrap();
        assert!(set.file_write_override);
    }

    #[test]
    fn hex_input_parser_is_bounded_and_strict() {
        assert_eq!(decode_hex("00aBff"), Some(vec![0, 0xab, 0xff]));
        assert_eq!(decode_hex(""), Some(vec![]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
        assert_eq!(decode_hex(&"0".repeat(MAX_FRAME_BYTES * 2 + 2)), None);
    }

    #[test]
    fn paste_is_normalized_bounded_and_rejects_controls() {
        assert_eq!(normalize_paste("a\r\nb\rc\t"), Ok(b"a\nb\nc\t".to_vec()));
        assert_eq!(normalize_paste(&"x".repeat(4096)).unwrap().len(), 4096);
        assert!(normalize_paste(&"x".repeat(4097)).is_err());
        assert!(normalize_paste("a\u{1b}b").is_err());
        assert_eq!(
            normalize_paste(&"界".repeat(1366)).unwrap_err(),
            "paste must be at most 4096 UTF-8 bytes and contain no control characters"
        );
    }

    #[test]
    fn wire_event_and_view_shape_are_presentation_neutral() {
        use crate::world::WorldEventKind;
        let event = WorldEvent {
            sequence: 3,
            participant_id: 9,
            kind: WorldEventKind::Chat("hello".to_owned()),
        };
        let encoded = serde_json::to_string(&ServerFrame::Event { event: &event }).unwrap();
        assert!(encoded.contains("\"type\":\"event\""));
        assert!(encoded.contains("\"sequence\":3"));
        assert!(encoded.contains("\"kind\":\"chat\""));
        assert!(encoded.contains("\"data\":\"hello\""));
        let view = serde_json::to_string(&ServerFrame::View {
            sequence: 3,
            data: "count=1",
            surface: None,
            view_revision: 0,
            base_view_revision: None,
            patch: None,
        })
        .unwrap();
        assert!(view.contains("\"type\":\"view\""));
        assert!(view.contains("\"data\":\"count=1\""));
    }

    #[test]
    fn typed_screen_is_an_optional_view_surface_without_dropping_legacy_data() {
        let data =
            r#"{"kind":"screen","version":1,"width":2,"height":1,"rows":["ok"],"styles":[[]]}"#;
        let surface = crate::world_presentation::parse_surface(data)
            .unwrap()
            .unwrap();
        let encoded = serde_json::to_string(&ServerFrame::View {
            sequence: 8,
            data,
            surface: Some(&surface),
            view_revision: 1,
            base_view_revision: None,
            patch: None,
        })
        .unwrap();
        assert!(encoded.contains("\"data\":\"{\\\"kind\\\":\\\"screen\\\""));
        assert!(encoded.contains("\"surface\":{\"kind\":\"screen\",\"version\":1"));
        assert!(encoded.contains("\"symbol\":\"o\""));
    }

    #[test]
    fn secret_comparison_requires_equal_bytes() {
        assert!(secret_eq(b"abc", b"abc"));
        assert!(!secret_eq(b"abc", b"abd"));
        assert!(!secret_eq(b"abc", b"ab"));
    }

    #[tokio::test]
    async fn silent_peers_are_dropped_after_the_join_timeout() {
        let mut peer = Peer::connect(
            service(None),
            SessionConfig {
                join_timeout: Duration::from_millis(50),
            },
        );
        let frame = peer.next().await.unwrap();
        assert_eq!(frame["type"], "error");
        assert!(frame["message"].as_str().unwrap().contains("timed out"));
        assert!(peer.next().await.is_none());
        peer.task.await.unwrap();
    }

    #[tokio::test]
    async fn first_frame_must_be_join_or_invite() {
        let mut peer = Peer::connect(service(Some("a")), SessionConfig::default());
        peer.say(serde_json::json!({"type": "chat", "text": "hi"}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");
        assert!(peer.next().await.is_none());

        let mut garbage = Peer::connect(service(None), SessionConfig::default());
        garbage
            .to_server
            .send(Inbound::Text("not json".to_owned()))
            .await
            .unwrap();
        assert_eq!(garbage.next().await.unwrap()["type"], "error");

        let mut big = Peer::connect(service(None), SessionConfig::default());
        big.to_server.send(Inbound::Oversized).await.unwrap();
        assert_eq!(big.next().await.unwrap()["type"], "error");
        let mut bin = Peer::connect(service(None), SessionConfig::default());
        bin.to_server.send(Inbound::Unsupported).await.unwrap();
        assert_eq!(bin.next().await.unwrap()["type"], "error");
    }

    #[test]
    fn invite_lifetimes_default_to_a_week_and_stop_at_thirty_days() {
        let now = 1_000_000;
        let default = invite_bounds(None, None, now).unwrap();
        assert_eq!(default.expires_at, Some(now + INVITE_DEFAULT_SECS * 1000));
        assert_eq!(default.uses, None);
        let longest = invite_bounds(Some(INVITE_MAX_SECS), Some(3), now).unwrap();
        assert_eq!(longest.expires_at, Some(now + INVITE_MAX_SECS * 1000));
        assert_eq!(longest.uses, Some(3));
        assert!(invite_bounds(Some(0), None, now).is_err());
        assert!(invite_bounds(Some(INVITE_MAX_SECS + 1), None, now).is_err());
        assert!(invite_bounds(None, Some(0), now).is_err());
    }

    #[tokio::test]
    async fn invite_frame_refuses_bounds_out_of_range() {
        let svc = service(Some("admin"));
        let mut long = Peer::connect(svc.clone(), SessionConfig::default());
        long.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann","expires_in_secs":INVITE_MAX_SECS + 1}))
            .await;
        assert_eq!(long.next().await.unwrap()["type"], "error");
        let mut none = Peer::connect(svc, SessionConfig::default());
        none.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann","uses":0}))
            .await;
        assert_eq!(
            none.next().await.unwrap()["message"],
            "uses must be at least 1"
        );
    }

    #[tokio::test]
    async fn delegate_to_friend_frame_needs_an_account_capability() {
        let svc = service(Some("admin"));
        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann"}))
            .await;
        let capability = host.next().await.unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();

        let mut guest = Peer::connect(svc, SessionConfig::default());
        guest
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(guest.next().await.unwrap()["type"], "snapshot");
        guest
            .say(serde_json::json!({"type":"delegate_to_friend","friend":"nobody","name":"bo","scopes":["join"]}))
            .await;
        assert_eq!(guest.next().await.unwrap()["message"], "delegation denied");
        guest
            .say(serde_json::json!({"type":"delegate_to_friend","friend":"nobody","name":"bo","scopes":["root"]}))
            .await;
        assert_eq!(guest.next().await.unwrap()["message"], "unknown scope");
    }

    #[tokio::test]
    async fn invite_frame_issues_a_capability_that_can_join() {
        let svc = service(Some("admin"));

        let mut denied = Peer::connect(svc.clone(), SessionConfig::default());
        denied
            .say(serde_json::json!({"type":"invite","admin_token":"nope","display_name":"x"}))
            .await;
        assert_eq!(denied.next().await.unwrap()["message"], "invite denied");

        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann"}))
            .await;
        let invite = host.next().await.unwrap();
        assert_eq!(invite["type"], "invite");
        let capability = invite["capability"].as_str().unwrap().to_owned();

        let mut guest = Peer::connect(svc, SessionConfig::default());
        guest
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(guest.next().await.unwrap()["type"], "snapshot");
    }

    #[tokio::test]
    async fn invite_roles_grant_and_withhold_the_expected_scopes() {
        let svc = service(Some("admin"));

        // An unknown role is refused by name.
        let mut bad = Peer::connect(svc.clone(), SessionConfig::default());
        bad.say(serde_json::json!({
            "type": "invite",
            "admin_token": "admin",
            "display_name": "spec",
            "role": "telepath",
        }))
        .await;
        let message = bad.next().await.unwrap()["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(message.contains("unknown invite role"), "{message}");

        // A spectator may join and read, but not chat or submit input.
        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({
            "type": "invite",
            "admin_token": "admin",
            "display_name": "spec",
            "role": "spectator",
        }))
        .await;
        let capability = host.next().await.unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut spectator = Peer::connect(svc.clone(), SessionConfig::default());
        spectator
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(spectator.next().await.unwrap()["type"], "snapshot");
        spectator
            .say(serde_json::json!({"type":"chat","text":"hello"}))
            .await;
        let refusal = spectator.next().await.unwrap();
        assert_eq!(refusal["type"], "error", "{refusal}");
        spectator
            .say(serde_json::json!({"type":"input","data_hex":"6c"}))
            .await;
        let refusal = spectator.next().await.unwrap();
        assert_eq!(refusal["type"], "error", "{refusal}");

        // A player may submit input but not chat.
        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({
            "type": "invite",
            "admin_token": "admin",
            "display_name": "player",
            "role": "player",
        }))
        .await;
        let capability = host.next().await.unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut player = Peer::connect(svc, SessionConfig::default());
        player
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(player.next().await.unwrap()["type"], "snapshot");
        player
            .say(serde_json::json!({"type":"input","data_hex":"6c"}))
            .await;
        let event = player.next().await.unwrap();
        assert_eq!(event["type"], "event", "{event}");
        player
            .say(serde_json::json!({"type":"chat","text":"hello"}))
            .await;
        let refusal = player.next().await.unwrap();
        assert_eq!(refusal["type"], "error", "{refusal}");
    }

    #[tokio::test]
    async fn a_delegate_mints_narrower_capabilities_over_the_session() {
        let svc = service(None);
        let delegate = WorldScopes::from_names(["join", "chat", "input", "delegate"]).unwrap();
        let (_, owner) = svc.world().issue("ann".to_owned(), delegate).await.unwrap();

        let mut owner_peer = Peer::connect(svc.clone(), SessionConfig::default());
        owner_peer
            .say(serde_json::json!({"type":"join","capability":owner.expose()}))
            .await;
        assert_eq!(owner_peer.next().await.unwrap()["type"], "snapshot");
        owner_peer
            .say(serde_json::json!({"type":"attenuate","name":"bo","scopes":["join"]}))
            .await;
        let delegated = owner_peer.next().await.unwrap();
        assert_eq!(delegated["type"], "invite");
        let child = delegated["capability"].as_str().unwrap().to_owned();

        let mut child_peer = Peer::connect(svc, SessionConfig::default());
        child_peer
            .say(serde_json::json!({"type":"join","capability":child}))
            .await;
        assert_eq!(child_peer.next().await.unwrap()["type"], "snapshot");
        child_peer
            .say(serde_json::json!({"type":"attenuate","name":"cy","scopes":["join"]}))
            .await;
        assert_eq!(child_peer.next().await.unwrap()["type"], "error");
        child_peer
            .say(serde_json::json!({"type":"attenuate","name":"cy","scopes":["telepathy"]}))
            .await;
        assert_eq!(child_peer.next().await.unwrap()["message"], "unknown scope");
    }

    /// A joined participant that may delegate `join`.
    async fn delegator(svc: &WorldService) -> Peer {
        let delegate = WorldScopes::from_names(["join", "chat", "input", "delegate"]).unwrap();
        let (_, owner) = svc.world().issue("ann".to_owned(), delegate).await.unwrap();
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type":"join","capability":owner.expose()}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "snapshot");
        peer
    }

    /// Delegation frames that name `uses`, in each position a client could put
    /// it. Serde would ignore the field and mint an unbounded grant.
    const FRAMES_NAMING_USES: [&str; 9] = [
        r#"{"type":"attenuate","uses":1,"name":"bo","scopes":["join"]}"#,
        r#"{"type":"attenuate","name":"bo","uses":1,"scopes":["join"]}"#,
        r#"{"type":"attenuate","name":"bo","scopes":["join"],"uses":1}"#,
        r#"{"type":"attenuate","name":"bo","scopes":["join"],"expires_at":null,"uses":null}"#,
        r#"{"type":"attenuate","name":"bo","scopes":["join"],"bounds":{"uses":1}}"#,
        r#"{"type":"attenuate","name":"bo","scopes":[{"uses":1}]}"#,
        r#"{"type":"delegate_to_friend","uses":1,"friend":"nobody","name":"bo","scopes":["join"]}"#,
        r#"{"type":"delegate_to_friend","friend":"nobody","name":"bo","scopes":["join"],"uses":1}"#,
        r#"{"type":"delegate_to_friend","friend":"nobody","name":"bo","scopes":["join"],"bounds":{"uses":1}}"#,
    ];

    #[tokio::test]
    async fn a_delegation_frame_that_names_uses_is_refused_by_name() {
        let svc = service(None);
        let mut peer = delegator(&svc).await;
        for text in FRAMES_NAMING_USES {
            peer.say_text(text).await;
            let reply = peer.next().await.unwrap();
            assert_eq!(reply["type"], "error", "{text}");
            assert!(
                reply["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("`uses`"),
                "{text}: {reply}"
            );
        }
    }

    #[tokio::test]
    async fn a_delegation_frame_without_uses_is_accepted() {
        let svc = service(None);
        let mut peer = delegator(&svc).await;
        for text in [
            r#"{"type":"attenuate","name":"bo","scopes":["join"]}"#,
            r#"{"type":"attenuate","scopes":["join"],"expires_at":null,"name":"cy"}"#,
        ] {
            peer.say_text(text).await;
            assert_eq!(peer.next().await.unwrap()["type"], "invite", "{text}");
        }
        // Past the uses check, this delegate has no account, so the friend
        // delegation is denied for that reason alone.
        peer.say_text(
            r#"{"type":"delegate_to_friend","friend":"nobody","name":"di","scopes":["join"]}"#,
        )
        .await;
        assert_eq!(peer.next().await.unwrap()["message"], "delegation denied");
    }

    #[tokio::test]
    async fn records_are_paged_prefixed_and_queryable_while_joined() {
        let svc = service_with_records(None);
        let (_, token) = svc
            .world()
            .issue("ann".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();

        // A fresh session can page through records; pages must be bounded and
        // terminate with `next_after: null`.
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        let mut pages = 0;
        let mut seen = crate::world::Records::new();
        let mut after: Option<String> = None;
        loop {
            let mut frame = serde_json::json!({
                "type": "records",
                "capability": token.expose(),
            });
            if let Some(after) = &after {
                frame["after"] = serde_json::Value::from(after.clone());
            }
            peer.say(frame).await;
            let reply = peer.next().await.unwrap();
            assert_eq!(reply["type"], "records");
            assert!(reply["sequence"].as_u64().is_some());
            let page: crate::world::Records =
                serde_json::from_value(reply["records"].clone()).unwrap();
            assert!(!page.is_empty());
            assert!(
                serde_json::to_vec(&page).unwrap().len() <= MAX_RECORDS_PAGE_BYTES,
                "page exceeds the byte budget"
            );
            pages += 1;
            seen.extend(page);
            match reply["next_after"].as_str() {
                Some(next) => after = Some(next.to_owned()),
                None => break,
            }
        }
        assert_eq!(seen.len(), 1024, "every record arrives exactly once");
        assert!(pages > 1, "1 MiB of records cannot fit in one page");
        assert!(seen.contains_key("key-0000") && seen.contains_key("key-1023"));

        // Prefix filtering works on the same session.
        peer.say(serde_json::json!({
            "type": "records",
            "capability": token.expose(),
            "prefix": "key-010",
        }))
        .await;
        let reply = peer.next().await.unwrap();
        let page: crate::world::Records = serde_json::from_value(reply["records"].clone()).unwrap();
        assert_eq!(page.len(), 10, "key-0100..key-0109");

        // A joined session can also request the records document ticket (the
        // reply is an error here only because this service has no docs store).
        let mut joined = Peer::connect(svc, SessionConfig::default());
        joined
            .say(serde_json::json!({"type":"join","capability": token.expose()}))
            .await;
        assert_eq!(joined.next().await.unwrap()["type"], "snapshot");
        joined
            .say(serde_json::json!({"type":"records","capability": token.expose()}))
            .await;
        let reply = joined.next().await.unwrap();
        assert_eq!(reply["type"], "records");
        let page: crate::world::Records = serde_json::from_value(reply["records"].clone()).unwrap();
        assert!(!page.is_empty() && page.len() < 1024, "one bounded page");
        assert!(
            reply["next_after"].is_string(),
            "the page advertises that more records follow"
        );
        // Still joined afterwards.
        joined
            .say(serde_json::json!({"type":"chat","text":"hi"}))
            .await;
        let reply = joined.next().await.unwrap();
        assert_eq!(reply["type"], "event");
    }

    fn memory_hub(
        limits: crate::world_hub::HubLimits,
    ) -> (WorldHub, Arc<crate::world_hub::testing::MemoryOpener>) {
        let opener = crate::world_hub::testing::MemoryOpener::new(false);
        let dyn_opener: Arc<dyn crate::world_hub::WorldOpener> = opener.clone();
        (
            WorldHub::new(dyn_opener, Some("admin".to_owned()), "lobby", limits),
            opener,
        )
    }

    /// Invite `name` into `world` (creating it) and return the capability.
    async fn invite_into(hub: &WorldHub, world: &str, name: &str) -> String {
        let mut peer = Peer::connect_hub(hub.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "invite",
            "admin_token": "admin",
            "display_name": name,
            "world": world,
        }))
        .await;
        let reply = peer.next().await.unwrap();
        assert_eq!(reply["type"], "invite", "{reply}");
        reply["capability"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn the_world_field_routes_a_session_and_capabilities_do_not_cross_worlds() {
        let (hub, _) = memory_hub(crate::world_hub::HubLimits::default());
        let in_arena = invite_into(&hub, "arena", "ann").await;
        let in_bazaar = invite_into(&hub, "bazaar", "bob").await;

        // Joining the world the capability belongs to works...
        let mut arena = Peer::connect_hub(hub.clone(), SessionConfig::default());
        arena
            .say(serde_json::json!({"type":"join","capability": in_arena,"world":"arena"}))
            .await;
        let snapshot = arena.next().await.unwrap();
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["snapshot"]["world_id"], "arena");

        // ...the same capability is refused in another world...
        let mut wrong = Peer::connect_hub(hub.clone(), SessionConfig::default());
        wrong
            .say(serde_json::json!({"type":"join","capability": in_arena,"world":"bazaar"}))
            .await;
        assert_eq!(wrong.next().await.unwrap()["message"], "join denied");

        // ...and chat in one world is invisible in the other.
        arena
            .say(serde_json::json!({"type":"chat","text":"hello arena"}))
            .await;
        assert_eq!(arena.next().await.unwrap()["type"], "event");
        let mut bazaar = Peer::connect_hub(hub.clone(), SessionConfig::default());
        bazaar
            .say(serde_json::json!({"type":"join","capability": in_bazaar,"world":"bazaar"}))
            .await;
        let snapshot = bazaar.next().await.unwrap();
        assert_eq!(snapshot["snapshot"]["world_id"], "bazaar");
        assert_eq!(snapshot["snapshot"]["events"].as_array().unwrap().len(), 0);

        // No `world` field means the default world, which nobody created.
        let mut default = Peer::connect_hub(hub, SessionConfig::default());
        default
            .say(serde_json::json!({"type":"join","capability": in_arena}))
            .await;
        assert_eq!(default.next().await.unwrap()["message"], "unknown world");
    }

    #[tokio::test]
    async fn only_an_admin_creates_or_lists_worlds() {
        let (hub, _) = memory_hub(crate::world_hub::HubLimits::default());
        let mut denied = Peer::connect_hub(hub.clone(), SessionConfig::default());
        denied
            .say(serde_json::json!({"type":"create_world","admin_token":"nope","world":"arena"}))
            .await;
        assert_eq!(denied.next().await.unwrap()["message"], "admin denied");
        let mut denied = Peer::connect_hub(hub.clone(), SessionConfig::default());
        denied
            .say(serde_json::json!({"type":"list_worlds","admin_token":"nope"}))
            .await;
        assert_eq!(denied.next().await.unwrap()["message"], "admin denied");

        let mut admin = Peer::connect_hub(hub.clone(), SessionConfig::default());
        admin
            .say(serde_json::json!({"type":"create_world","admin_token":"admin","world":"arena"}))
            .await;
        let created = admin.next().await.unwrap();
        assert_eq!(
            (created["type"].as_str(), created["created"].as_bool()),
            (Some("world_created"), Some(true))
        );
        let mut admin = Peer::connect_hub(hub.clone(), SessionConfig::default());
        admin
            .say(serde_json::json!({"type":"create_world","admin_token":"admin","world":"arena"}))
            .await;
        assert_eq!(admin.next().await.unwrap()["created"], false);
        let mut admin = Peer::connect_hub(hub.clone(), SessionConfig::default());
        admin
            .say(serde_json::json!({"type":"create_world","admin_token":"admin","world":"../x"}))
            .await;
        assert_eq!(admin.next().await.unwrap()["message"], "invalid world name");

        let mut admin = Peer::connect_hub(hub, SessionConfig::default());
        admin
            .say(serde_json::json!({"type":"list_worlds","admin_token":"admin"}))
            .await;
        let listed = admin.next().await.unwrap();
        assert_eq!(listed["type"], "worlds");
        assert_eq!(listed["default"], "lobby");
        assert_eq!(listed["worlds"][0]["name"], "arena");
    }

    #[tokio::test]
    async fn unauthenticated_frames_cannot_create_or_probe_worlds() {
        let (hub, opener) = memory_hub(crate::world_hub::HubLimits::default());
        for frame in [
            serde_json::json!({"type":"join","capability":"1.abc","world":"arena"}),
            serde_json::json!({"type":"info","capability":"1.abc","world":"arena"}),
            serde_json::json!({"type":"invite","admin_token":"wrong","display_name":"x","world":"arena"}),
        ] {
            let mut peer = Peer::connect_hub(hub.clone(), SessionConfig::default());
            peer.say(frame).await;
            assert_eq!(peer.next().await.unwrap()["message"], "unknown world");
        }
        assert_eq!(
            opener.opens.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "nothing was opened or created by outsiders"
        );
    }

    #[tokio::test]
    async fn many_sessions_across_many_worlds_stay_separate() {
        let (hub, _) = memory_hub(crate::world_hub::HubLimits::default());
        let mut tasks = tokio::task::JoinSet::new();
        for world in 0..8 {
            for player in 0..8 {
                let hub = hub.clone();
                tasks.spawn(async move {
                    let name = format!("world-{world}");
                    let capability = invite_into(&hub, &name, &format!("p{player}")).await;
                    let mut peer = Peer::connect_hub(hub, SessionConfig::default());
                    peer.say(serde_json::json!({
                        "type": "join", "capability": capability, "world": name
                    }))
                    .await;
                    let snapshot = peer.next().await.unwrap();
                    assert_eq!(snapshot["snapshot"]["world_id"], name.as_str());
                    peer.say(serde_json::json!({
                        "type": "chat", "text": format!("{name}/{player}")
                    }))
                    .await;
                    // Every event this session sees was said in its own world.
                    let mut seen = 0;
                    while seen < 1 {
                        let frame = peer.next().await.unwrap();
                        if frame["type"] == "event" {
                            let text = frame["event"]["kind"]["data"].as_str().unwrap_or_default();
                            assert!(text.starts_with(&name), "{name} saw {text}");
                            seen += 1;
                        }
                    }
                });
            }
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(hub.open_count(), 8);
    }

    #[tokio::test]
    async fn a_session_over_the_ceiling_is_refused_politely() {
        let (hub, _) = memory_hub(crate::world_hub::HubLimits {
            max_sessions: 1,
            ..crate::world_hub::HubLimits::default()
        });
        let capability = invite_into(&hub, "arena", "ann").await;
        let mut first = Peer::connect_hub(hub.clone(), SessionConfig::default());
        first
            .say(serde_json::json!({"type":"join","capability": capability,"world":"arena"}))
            .await;
        assert_eq!(first.next().await.unwrap()["type"], "snapshot");
        let mut second = Peer::connect_hub(hub, SessionConfig::default());
        second
            .say(serde_json::json!({"type":"join","capability": capability,"world":"arena"}))
            .await;
        assert_eq!(
            second.next().await.unwrap()["message"],
            "server is busy; try again shortly"
        );
    }

    #[tokio::test]
    async fn records_queries_need_a_valid_capability() {
        let svc = service_with_records(None);
        let mut peer = Peer::connect(svc, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "records",
            "capability": "0.00",
        }))
        .await;
        assert_eq!(peer.next().await.unwrap()["message"], "join denied");
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn admin_upload_pins_validates_and_activates_a_roc_program() {
        use iroh_blobs::store::mem::MemStore;

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let blobs: Store = MemStore::new().into();
        let svc = WorldService::new(world.clone(), Some("admin".to_owned()))
            .with_blob_store(blobs.clone());
        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm");
        let hash = module_hash(wasm);

        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "install_begin",
            "admin_token": "wrong",
            "total_bytes": wasm.len(),
            "module_hash": hash,
            "seed": 12,
        }))
        .await;
        assert_eq!(
            peer.next().await.unwrap()["message"],
            "module install denied"
        );

        let (_, guest) = world
            .issue("ada".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let mut peer = Peer::connect(svc, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "install_begin",
            "admin_token": "admin",
            "total_bytes": wasm.len(),
            "module_hash": null,
            "seed": 12,
        }))
        .await;
        let ready = peer.next().await.unwrap();
        assert_eq!(ready["type"], "upload_ready");
        let chunk_bytes = usize::try_from(ready["chunk_bytes"].as_u64().unwrap()).unwrap();
        for (index, chunk) in wasm.chunks(chunk_bytes).enumerate() {
            peer.say(serde_json::json!({
                "type": "install_chunk",
                "offset": index * chunk_bytes,
                "data_hex": encode_hex(chunk),
            }))
            .await;
        }
        peer.say(serde_json::json!({"type": "install_end"})).await;
        let ack = peer.next().await.unwrap();
        assert_eq!(ack["type"], "module_installed");
        assert_eq!(ack["module_hash"], hash);

        let hash: Hash = hash.parse().unwrap();
        assert!(
            blobs.blobs().has(hash).await.unwrap(),
            "uploaded module is pinned in the blob store"
        );
        assert!(
            world
                .view(guest.clone())
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=0"))
        );
        world.input(guest.clone(), b"inc".to_vec()).await.unwrap();
        assert!(
            world
                .view(guest)
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=1"))
        );
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn admin_fuel_limit_applies_to_installed_programs() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm").to_vec();
        let hash = module_hash(&wasm);
        let starved = crate::world_limits::RuntimeLimits {
            sandbox: crate::sandbox::SandboxLimits {
                fuel: 1,
                ..crate::sandbox::SandboxLimits::default()
            },
            ..crate::world_limits::RuntimeLimits::default()
        };
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let svc = WorldService::new(world, Some("admin".to_owned()))
            .with_blob_store(MemStore::new().into())
            .with_runtime_limits(starved);
        assert!(matches!(
            svc.install_wasm(Some("admin"), wasm.clone(), 12, &hash)
                .await,
            Err(InstallError::InvalidModule)
        ));

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let svc = WorldService::new(world, Some("admin".to_owned()))
            .with_blob_store(MemStore::new().into());
        svc.set_execution_profile(
            Some("admin"),
            crate::world_limits::WorldExecutionProfile {
                wasm_fuel: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            svc.install_wasm(Some("admin"), wasm.clone(), 12, &hash)
                .await,
            Err(InstallError::InvalidModule)
        ));

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let svc = WorldService::new(world, Some("admin".to_owned()))
            .with_blob_store(MemStore::new().into());
        svc.install_wasm(Some("admin"), wasm, 12, &hash)
            .await
            .expect("default limits install");
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn source_only_policy_rejects_raw_modules_but_preserves_normal_mode() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm").to_vec();
        let hash = module_hash(&wasm);
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world.clone(), Some("admin".to_owned()))
            .with_blob_store(MemStore::new().into())
            .with_source_only(true);

        assert!(service.source_only());
        assert!(matches!(
            service
                .install_wasm(Some("wrong"), wasm.clone(), 1, &hash)
                .await,
            Err(InstallError::Unauthorized)
        ));
        assert!(matches!(
            service
                .install_wasm(Some("admin"), wasm.clone(), 1, &hash)
                .await,
            Err(InstallError::RawModuleDisabled)
        ));
        assert_eq!(service.active_module_hash().await, None);

        let normal = WorldService::new(
            WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
            Some("admin".to_owned()),
        )
        .with_blob_store(MemStore::new().into());
        normal
            .install_wasm(Some("admin"), wasm, 1, &hash)
            .await
            .expect("raw modules stay supported by default");
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn module_trust_list_allows_empty_refuses_unsigned_and_accepts_trusted_signer() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm").to_vec();
        let hash = module_hash(&wasm);

        let empty = WorldService::new(
            WorldHandle::spawn(WorldCore::new("empty-trust", WorldLimits::default()).unwrap()),
            Some("admin".to_owned()),
        )
        .with_blob_store(MemStore::new().into());
        empty
            .install_wasm(Some("admin"), wasm.clone(), 1, &hash)
            .await
            .expect("empty trust list keeps legacy installs working");

        let signer = SigningKey::from_bytes(&[42; 32]);
        let trusted = WorldService::new(
            WorldHandle::spawn(WorldCore::new("trusted", WorldLimits::default()).unwrap()),
            Some("admin".to_owned()),
        )
        .with_blob_store(MemStore::new().into())
        .with_module_trust_keys(vec![signer.verifying_key().to_bytes()]);
        assert_eq!(
            trusted
                .install_wasm(Some("admin"), wasm.clone(), 1, &hash)
                .await,
            Err(InstallError::UntrustedModule),
            "a non-empty trust list refuses unsigned uploads"
        );

        let mut message = b"developing-today/id-world-module/v1\0".to_vec();
        message.extend_from_slice(blake3::hash(&wasm).as_bytes());
        let signature = ModuleSignature {
            public_key: hex_encode(&signer.verifying_key().to_bytes()),
            signature: hex_encode(&signer.sign(&message).to_bytes()),
        };
        trusted
            .install_wasm_with_signature(Some("admin"), wasm, 2, &hash, Some(&signature))
            .await
            .expect("a trust-listed signer may install the module");
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn source_only_policy_rejects_streaming_upload_before_accepting_chunks() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, Some("admin".to_owned())).with_source_only(true);
        let mut peer = Peer::connect(service, SessionConfig::default());
        peer.say(serde_json::json!({
            "type": "install_begin",
            "admin_token": "admin",
            "total_bytes": 32,
            "module_hash": null,
            "seed": 1,
        }))
        .await;
        let error = peer.next().await.unwrap();
        assert_eq!(error["type"], "error");
        assert!(error["message"].as_str().unwrap().contains("source-only"));
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn bad_module_replacement_preserves_the_active_program() {
        use iroh_blobs::store::mem::MemStore;

        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm");
        let sandbox =
            crate::sandbox::Sandbox::compile(wasm, crate::sandbox::SandboxLimits::default())
                .unwrap();
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("counter", WorldLimits::default()).unwrap(),
            Box::new(sandbox.instantiate(1).unwrap()),
        );
        let blobs: Store = MemStore::new().into();
        let service =
            WorldService::new(world.clone(), Some("admin".to_owned())).with_blob_store(blobs);
        let (_, guest) = world.issue("guest", WorldScopes::GUEST).await.unwrap();
        assert!(
            world
                .view(guest.clone())
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=0"))
        );

        let bad = b"not wasm".to_vec();
        let hash = module_hash(&bad);
        assert_eq!(
            service
                .install_wasm(Some("admin"), bad, 2, &hash)
                .await
                .unwrap_err(),
            InstallError::InvalidModule
        );
        assert!(
            world
                .view(guest.clone())
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=0"))
        );
        world.input(guest.clone(), b"inc".to_vec()).await.unwrap();
        assert!(
            world
                .view(guest)
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=1"))
        );
    }

    #[tokio::test]
    async fn invites_are_disabled_without_an_admin_secret() {
        let mut peer = Peer::connect(service(None), SessionConfig::default());
        peer.say(serde_json::json!({"type":"invite","admin_token":"","display_name":"x"}))
            .await;
        assert_eq!(
            peer.next().await.unwrap()["message"],
            "invites are not enabled"
        );
    }

    #[tokio::test]
    async fn chat_is_broadcast_and_rejections_are_private() {
        let svc = service(None);
        let (_, alice) = svc
            .world()
            .issue("alice".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, watcher) = svc
            .world()
            .issue("watcher".to_owned(), WorldScopes::JOIN)
            .await
            .unwrap();
        let mut a = Peer::connect(svc.clone(), SessionConfig::default());
        let mut w = Peer::connect(svc, SessionConfig::default());
        a.say(serde_json::json!({"type":"join","capability":alice.expose()}))
            .await;
        w.say(serde_json::json!({"type":"join","capability":watcher.expose()}))
            .await;
        assert_eq!(a.next().await.unwrap()["type"], "snapshot");
        assert_eq!(w.next().await.unwrap()["type"], "snapshot");

        // A join-only participant cannot speak; only they hear about it.
        w.say(serde_json::json!({"type":"chat","text":"psst"}))
            .await;
        assert_eq!(w.next().await.unwrap()["message"], "world event rejected");

        // Chat reaches the guest; the join-only watcher hears the next
        // non-chat event instead, past a gap in sequence numbers.
        a.say(serde_json::json!({"type":"chat","text":"hello"}))
            .await;
        let frame = a.next().await.unwrap();
        assert_eq!(frame["type"], "event");
        assert!(frame.to_string().contains("hello"));
        a.say(serde_json::json!({"type":"input","data_hex":"00"}))
            .await;
        let frame = w.next().await.unwrap();
        assert_eq!(frame["type"], "event");
        assert_eq!(frame["event"]["sequence"], 2, "{frame}");
        assert!(!frame.to_string().contains("hello"), "{frame}");
        assert_eq!(a.next().await.unwrap()["event"]["sequence"], 2);
        a.say(serde_json::json!({"type":"input","data_hex":"xyz"}))
            .await;
        assert_eq!(
            a.next().await.unwrap()["message"],
            "input data must be hexadecimal"
        );
        a.say(serde_json::json!({"type":"join","capability":"x"}))
            .await;
        assert_eq!(a.next().await.unwrap()["message"], "already joined");
    }

    fn kinds(events: &serde_json::Value) -> Vec<String> {
        events
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["kind"]["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn chat_visibility_a_join_only_joiner_gets_a_snapshot_without_chat() {
        let svc = service(None);
        let world = svc.world();
        let (_, guest) = world
            .issue("guest".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, join_only) = world
            .issue("join".to_owned(), WorldScopes::JOIN)
            .await
            .unwrap();
        let (_, reader) = world
            .issue(
                "reader".to_owned(),
                WorldScopes::from_names(["join", "history"]).unwrap(),
            )
            .await
            .unwrap();
        world
            .chat(guest.clone(), "secret".to_owned())
            .await
            .unwrap();
        world.input(guest.clone(), b"move".to_vec()).await.unwrap();

        for (token, sees_chat) in [(&guest, true), (&reader, true), (&join_only, false)] {
            let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
            peer.say(serde_json::json!({"type":"join","capability":token.expose()}))
                .await;
            let frame = peer.next().await.unwrap();
            assert_eq!(frame["type"], "snapshot");
            let events = &frame["snapshot"]["events"];
            assert_eq!(frame["snapshot"]["current_sequence"], 2);
            assert_eq!(
                kinds(events),
                if sees_chat {
                    vec!["chat", "input"]
                } else {
                    vec!["input"]
                },
                "{frame}"
            );
            assert_eq!(frame.to_string().contains("secret"), sees_chat, "{frame}");
        }

        // A rejoin from a cursor goes through catch-up, which projects too.
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type":"join","capability":join_only.expose(),"after":0}))
            .await;
        let frame = peer.next().await.unwrap();
        assert_eq!(frame["type"], "event", "{frame}");
        assert_eq!(frame["event"]["sequence"], 2);
        assert!(!frame.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn chat_visibility_a_live_install_never_shows_a_session_the_seed() {
        // The raw broadcast carries the real seed; the session sends a
        // snapshot and a program-changed frame instead, and every frame it
        // sends is built from the projection.
        let svc = service(None);
        let world = svc.world();
        let (_, guest) = world
            .issue("guest".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let mut peer = Peer::connect(svc.clone(), SessionConfig::default());
        peer.say(serde_json::json!({"type":"join","capability":guest.expose()}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "snapshot");
        world
            .install_program("ab".repeat(32), 424_242, Box::new(NoProgram))
            .await
            .unwrap();
        let mut frames = Vec::new();
        loop {
            let frame = peer.next().await.unwrap();
            let done = frame["type"] == "program_changed";
            frames.push(frame);
            if done {
                break;
            }
        }
        let text = serde_json::to_string(&frames).unwrap();
        assert!(!text.contains("424242"), "{text}");
        assert!(text.contains(&"ab".repeat(32)), "the module hash is kept");
    }

    #[tokio::test]
    async fn chat_visibility_the_live_stream_follows_the_viewers_scopes() {
        let svc = service(None);
        let world = svc.world();
        let (_, guest) = world
            .issue("guest".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, join_only) = world
            .issue("join".to_owned(), WorldScopes::JOIN)
            .await
            .unwrap();
        let (_, reader) = world
            .issue(
                "reader".to_owned(),
                WorldScopes::from_names(["join", "history"]).unwrap(),
            )
            .await
            .unwrap();
        let mut watcher = Peer::connect(svc.clone(), SessionConfig::default());
        let mut reading = Peer::connect(svc.clone(), SessionConfig::default());
        for (peer, token) in [(&mut watcher, &join_only), (&mut reading, &reader)] {
            peer.say(serde_json::json!({"type":"join","capability":token.expose()}))
                .await;
            assert_eq!(peer.next().await.unwrap()["type"], "snapshot");
        }

        world
            .chat(guest.clone(), "secret".to_owned())
            .await
            .unwrap();
        world.input(guest.clone(), b"move".to_vec()).await.unwrap();

        let frame = reading.next().await.unwrap();
        assert!(frame.to_string().contains("secret"), "{frame}");
        assert_eq!(reading.next().await.unwrap()["event"]["sequence"], 2);
        // The join-only watcher's first event is the input, after the gap.
        let frame = watcher.next().await.unwrap();
        assert_eq!(frame["type"], "event", "{frame}");
        assert_eq!(frame["event"]["sequence"], 2, "{frame}");
        assert!(!frame.to_string().contains("secret"));

        // A reader may not speak.
        reading
            .say(serde_json::json!({"type":"chat","text":"hi"}))
            .await;
        assert_eq!(
            reading.next().await.unwrap()["message"],
            "world event rejected"
        );
    }
}
