//! Acceptance for Bevy slice 4 (#13705): the presenter-client app joins
//! a world hosted in another process, projects it, submits intents end
//! to end, shows predicted presentation that reconciles or snaps back,
//! and produces only the surfaces the world grants.
//!
//! Every test starts the demo world as a separate `id-bevy-presenter
//! host` process, so the append gate is outside the presenter's process.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;

use id_bevy_presenter::host::OUTSIDE_ARENA;
use id_bevy_presenter::remote::{Reply, Request, spawn_host};
use id_bevy_presenter::{
    AVATAR, Cue, PresentedFrame, PresenterEvent, RemoteError, RemoteWorld, ScriptRun,
    device_manifest, parse_script, run_script,
};

const PROGRAM: &str = env!("CARGO_BIN_EXE_id-bevy-presenter");

/// A host process granting `grant` (every surface when `None`).
struct Host {
    child: Child,
    link: Arc<RemoteWorld>,
}

impl Host {
    fn start(grant: Option<&str>) -> Self {
        let mut args = vec!["host".to_owned()];
        if let Some(grant) = grant {
            args.push("--grant".to_owned());
            args.push(grant.to_owned());
        }
        let (child, link) = spawn_host(Path::new(PROGRAM), &args).unwrap();
        Self {
            child,
            link: Arc::new(link),
        }
    }

    fn run(self, script: &str) -> ScriptRun {
        let presses = parse_script(script).unwrap();
        let run = run_script(&self.link, &presses).unwrap();
        self.stop();
        run
    }

    fn stop(mut self) {
        drop(self.link);
        // Closing the link ends the host's input; it exits by itself.
        let status = self.child.wait().unwrap();
        assert!(status.success(), "host exited with {status}");
    }
}

fn avatar(frame: &PresentedFrame) -> ([f64; 2], bool) {
    let drawn = frame.entity(AVATAR).unwrap();
    (drawn.at, drawn.predicted)
}

#[test]
fn it_joins_a_world_in_another_process_and_projects_it() {
    let run = Host::start(None).run("");
    assert_eq!(run.surfaces, ["render", "input", "audio_out"]);
    assert_eq!(run.replay, Ok(()));
    let last = run.frames.last().unwrap();
    assert_eq!(last.applied, Some(2));
    let ids: Vec<&str> = last
        .entities
        .iter()
        .map(|drawn| drawn.id.as_str())
        .collect();
    assert_eq!(ids, ["avatar-you", "beacon-east", "beacon-north"]);
    assert_eq!(avatar(last), ([0.0, 0.0], false));
    assert_eq!(
        last.entity("beacon-north").unwrap().name.as_deref(),
        Some("North beacon")
    );
    // One sprite per entity, placed in pixels from world units.
    assert_eq!(
        run.sprites,
        [
            ("avatar-you".to_owned(), [0.0, 0.0]),
            ("beacon-east".to_owned(), [96.0, 0.0]),
            ("beacon-north".to_owned(), [0.0, 96.0]),
        ]
    );
    assert!(run.events.is_empty());
}

#[test]
fn an_accepted_move_is_drawn_predicted_then_reconciled() {
    let run = Host::start(None).run("R");
    assert_eq!(
        run.events,
        [
            PresenterEvent::Predicted {
                intent_id: 1,
                at: [1.0, 0.0],
            },
            PresenterEvent::Reconciled {
                intent_id: 1,
                position: 3,
                at: [1.0, 0.0],
            },
        ]
    );
    // The prediction is drawn while the world still shows position 2,
    // then the same place is drawn as world state at position 3.
    let predicted = run.frames.iter().position(|frame| avatar(frame).1).unwrap();
    assert_eq!(run.frames[predicted].applied, Some(2));
    assert_eq!(avatar(&run.frames[predicted]), ([1.0, 0.0], true));
    let after = &run.frames[predicted + 1];
    assert_eq!(after.applied, Some(3));
    assert_eq!(avatar(after), ([1.0, 0.0], false));
    assert_eq!(run.cues, [Cue::Confirm]);
    assert_eq!(run.replay, Ok(()));
}

#[test]
fn a_rejected_move_visibly_snaps_back() {
    // The arena ends at x = 3: the fourth step right is refused.
    let run = Host::start(None).run("RRRR");
    let snapped = run
        .events
        .iter()
        .find_map(|event| match event {
            PresenterEvent::SnappedBack {
                intent_id,
                from,
                to,
                reason,
            } => Some((*intent_id, *from, *to, reason.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        snapped,
        (4, [4.0, 0.0], [3.0, 0.0], OUTSIDE_ARENA.to_owned())
    );

    // A frame draws the refused step, and the very next frame draws the
    // avatar back at its applied position.
    let predicted = run
        .frames
        .iter()
        .position(|frame| avatar(frame) == ([4.0, 0.0], true))
        .unwrap();
    assert_eq!(avatar(&run.frames[predicted + 1]), ([3.0, 0.0], false));
    // Nothing was journaled for the refused step.
    assert_eq!(run.frames.last().unwrap().applied, Some(5));
    assert_eq!(
        run.cues,
        [Cue::Confirm, Cue::Confirm, Cue::Confirm, Cue::SnapBack]
    );
    assert_eq!(run.replay, Ok(()));
}

#[test]
fn the_gamepad_drives_the_same_intents_as_the_keyboard() {
    let run = Host::start(None).run("ul");
    let reconciled: Vec<[f64; 2]> = run
        .events
        .iter()
        .filter_map(|event| match event {
            PresenterEvent::Reconciled { at, .. } => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(reconciled, [[0.0, 1.0], [-1.0, 1.0]]);
    assert_eq!(run.replay, Ok(()));
}

#[test]
fn only_the_surfaces_the_world_grants_are_produced() {
    // Render only: the world is drawn, input is ignored, no cue sounds.
    let run = Host::start(Some("Screen")).run("R");
    assert_eq!(run.surfaces, ["render"]);
    assert!(run.events.is_empty());
    assert_eq!(run.sprites.len(), 3);
    assert!(run.cues.is_empty());
    assert_eq!(run.frames.last().unwrap().applied, Some(2));

    // Input and audio without render: moves still go through the gate,
    // but nothing is drawn.
    let run = Host::start(Some("Key,speech.synthesize")).run("R");
    assert_eq!(run.surfaces, ["input", "audio_out"]);
    assert!(run.sprites.is_empty());
    assert_eq!(run.cues, [Cue::Confirm]);
    assert_eq!(run.frames.last().unwrap().applied, Some(3));
    assert_eq!(run.replay, Ok(()));
}

#[test]
fn a_world_granting_an_unknown_provider_is_refused() {
    let host = Host::start(Some("Screne"));
    let error = run_script(&host.link, &[]).unwrap_err();
    assert!(matches!(error, RemoteError::Refused(_)), "{error}");
    host.stop();
}

#[test]
fn a_presenter_joins_only_with_a_valid_device_manifest() {
    let host = Host::start(None);
    let read = Request::ReadAfter {
        after: None,
        max: 1,
    };
    // Nothing is served before hello.
    assert!(matches!(
        host.link.call(&read),
        Err(RemoteError::Refused(_))
    ));
    // A manifest of another version is refused by the standard parser.
    let mut wire = device_manifest(640, 480).to_json();
    wire["manifest"]["v"] = serde_json::json!(2);
    assert!(matches!(
        host.link.call(&Request::Hello { device: wire }),
        Err(RemoteError::Refused(_))
    ));
    // The app's own manifest is welcomed with the world's grants.
    let granted = host.link.hello(&device_manifest(640, 480)).unwrap();
    assert_eq!(granted, ["Screen", "Key", "speech.synthesize"]);
    match host.link.call(&read).unwrap() {
        Reply::Batches { batches } => assert_eq!(batches[0].position, 1),
        other => panic!("unexpected reply {other:?}"),
    }
    host.stop();
}

#[test]
fn the_binary_runs_a_script_against_its_own_host_process() {
    let output = Command::new(PROGRAM)
        .args(["script", "RRRR"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["replay"], serde_json::json!({ "Ok": null }));
    assert_eq!(report["events"].as_array().unwrap().len(), 8);
    assert_eq!(report["events"][7]["event"], "snapped_back");
    assert_eq!(report["cues"][3], "snap_back");

    let refused = Command::new(PROGRAM)
        .args(["script", "RX"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
}
