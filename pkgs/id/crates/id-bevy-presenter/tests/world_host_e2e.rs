//! Acceptance for Bevy slice 5 (#13706), the host posture: the Bevy
//! process also runs the world's append gate and its journal.
//!
//! - A second presenter, in this process over a socket or as the slice 4
//!   app in its own process, joins the Bevy-hosted world and sees exactly
//!   what it sees when the world is hosted elsewhere.
//! - The host's own presenter takes the same path as a remote one: the
//!   same projection, the same gate, the same outcomes.
//! - The journal outlives the host: reopening it resumes from its last
//!   position, and a killed host process loses no acknowledged position.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use id_bevy_presenter::host::{ArenaHost, OUTSIDE_ARENA};
use id_bevy_presenter::remote::{Reply, Request, spawn_host};
use id_bevy_presenter::world_host::WORLD_CLOSED;
use id_bevy_presenter::{
    AVATAR, Cue, EmbeddedHost, HostSettings, JournalStore, Listener, PresenterEvent, RemoteError,
    RemoteWorld, ScriptRun, Session, connect_tcp, device_manifest, listen, parse_script,
    run_script,
};

const PROGRAM: &str = env!("CARGO_BIN_EXE_id-bevy-presenter");

/// A fresh, empty directory for one test's journal.
fn journal_path(test: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("id-bevy-world-host-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("arena.journal")
}

/// The same script against the slice 4 host: the arena in its own
/// process, journal in memory.
fn run_against_an_external_host(script: &str) -> ScriptRun {
    let (mut child, link) = spawn_host(Path::new(PROGRAM), &["host".to_owned()]).unwrap();
    let link = Arc::new(link);
    let run = run_script(&link, &parse_script(script).unwrap()).unwrap();
    drop(link);
    assert!(child.wait().unwrap().success());
    run
}

/// The arena hosted in this process, its door open on a free port.
fn hosted_arena(path: &Path) -> (Arc<EmbeddedHost>, Listener) {
    let host = Arc::new(EmbeddedHost::arena(path, ArenaHost::every_surface()).unwrap());
    let door = listen(Arc::clone(&host), "127.0.0.1:0").unwrap();
    (host, door)
}

fn join(door: &Listener) -> Arc<RemoteWorld> {
    Arc::new(connect_tcp(door.addr()).unwrap())
}

#[test]
fn a_second_presenter_sees_the_bevy_hosted_world_as_it_sees_an_external_one() {
    let path = journal_path("interop");
    let (host, door) = hosted_arena(&path);
    let mut own = Session::host(&host).unwrap();

    // Walk right to the arena's edge and one step past it, then up and
    // back with the d-pad: accepted moves, a refused one, both inputs.
    let script = "RRRRud";
    let link = join(&door);
    let remote = run_script(&link, &parse_script(script).unwrap()).unwrap();
    let external = run_against_an_external_host(script);
    assert_eq!(remote, external);
    assert_eq!(remote.replay, Ok(()));
    assert!(remote.events.iter().any(|event| matches!(
        event,
        PresenterEvent::SnappedBack { reason, .. } if reason == OUTSIDE_ARENA
    )));

    // The host's own presenter applied the same positions and holds the
    // same world: what it draws is what the second presenter drew.
    own.settle();
    let own_run = own.finish(&host.journal());
    assert_eq!(own_run.replay, Ok(()));
    assert_eq!(own_run.sprites, remote.sprites);
    assert_eq!(
        own_run.frames.last().unwrap().applied,
        remote.frames.last().unwrap().applied
    );
}

#[test]
fn the_hosts_own_presenter_takes_the_same_path_as_a_remote_one() {
    let path = journal_path("own-path");
    let (host, door) = hosted_arena(&path);
    let mut own = Session::host(&host).unwrap();
    let link = join(&door);
    let mut watcher = Session::join(&link).unwrap();

    // The host plays; the remote presenter only watches.
    for press in parse_script("RRRR").unwrap() {
        own.press(press);
    }
    watcher.settle();

    // Its moves went through the gate like anyone's: three accepted, the
    // fourth refused at the arena's edge and left out of the journal.
    let positions: Vec<u64> = host.journal().iter().map(|b| b.position).collect();
    assert_eq!(positions, vec![1, 2, 3, 4, 5]);

    // Both presenters hold the same world, written only by the bridge.
    let own_state = own.world_state();
    let watched = watcher.world_state();
    assert_eq!(own_state, watched);
    assert_eq!(own_state.unmapped_world_state, 0);
    assert_eq!(own_state.applied, Some(5));
    assert_eq!(
        own_state.entities[AVATAR]
            .position
            .map(|p| [f64::from_bits(p.x_bits), f64::from_bits(p.y_bits)]),
        Some([3.0, 0.0])
    );

    // And the host's presenter showed exactly what a remote presenter
    // shows for the same presses against an external host.
    let own_run = own.finish(&host.journal());
    let external = run_against_an_external_host("RRRR");
    assert_eq!(own_run.events, external.events);
    assert_eq!(own_run.cues, external.cues);
    assert_eq!(own_run.sprites, external.sprites);
    assert_eq!(own_run.frames, external.frames);
    assert_eq!(own_run.replay, Ok(()));
    assert_eq!(watcher.finish(&host.journal()).replay, Ok(()));
}

#[test]
fn reopening_the_world_resumes_from_its_last_position() {
    let path = journal_path("reopen");
    let (host, door) = hosted_arena(&path);
    let link = join(&door);
    let first = run_script(&link, &parse_script("RR").unwrap()).unwrap();
    assert_eq!(first.replay, Ok(()));
    let before = host.journal();
    assert_eq!(before.last().map(|b| b.position), Some(4));
    drop(link);
    drop(door);
    host.close().unwrap();
    drop(host);

    // A new host on the same journal has every position, in order, and
    // the next accepted move takes the next position.
    let (host, door) = hosted_arena(&path);
    assert_eq!(host.journal(), before);
    let link = join(&door);
    let second = run_script(&link, &parse_script("U").unwrap()).unwrap();
    assert_eq!(second.replay, Ok(()));
    let reconciled: Vec<(u64, [f64; 2])> = second
        .events
        .iter()
        .filter_map(|event| match event {
            PresenterEvent::Reconciled { position, at, .. } => Some((*position, *at)),
            _ => None,
        })
        .collect();
    assert_eq!(reconciled, vec![(5, [2.0, 1.0])]);
    assert_eq!(second.cues, vec![Cue::Confirm]);
    drop(link);
    drop(door);
    host.close().unwrap();
    assert_eq!(JournalStore::open(&path).unwrap().head(), Some(5));
}

#[test]
fn admission_is_by_manifest_up_to_capacity_and_ends_when_the_world_closes() {
    let path = journal_path("admission");
    let mut settings = HostSettings::arena(ArenaHost::every_surface());
    settings.capacity = 1;
    let host = Arc::new(
        EmbeddedHost::create(&path, id_bevy_presenter::host::arena_genesis(), settings).unwrap(),
    );
    let door = listen(Arc::clone(&host), "127.0.0.1:0").unwrap();
    let device = device_manifest(640, 480);

    // Nothing is served before a manifest.
    let first = connect_tcp(door.addr()).unwrap();
    assert!(matches!(
        first.call(&Request::ReadAfter {
            after: None,
            max: 8
        }),
        Err(RemoteError::Refused(_))
    ));
    assert!(first.hello(&device).is_ok());
    assert_eq!(host.present(), 1);

    let second = connect_tcp(door.addr()).unwrap();
    let Err(RemoteError::Refused(reason)) = second.hello(&device) else {
        panic!("a presenter past capacity was admitted");
    };
    assert!(reason.contains("full"), "{reason}");

    // The first presenter leaves; its place is free again.
    drop(first);
    wait_until(|| host.present() == 0);
    assert!(second.hello(&device).is_ok());

    // A closed world admits no one and journals nothing, but its
    // journal stays readable.
    host.close().unwrap();
    let third = connect_tcp(door.addr()).unwrap();
    assert_eq!(
        third.hello(&device),
        Err(RemoteError::Refused(WORLD_CLOSED.to_owned()))
    );
    assert!(matches!(
        second.call(&Request::Submit {
            frame: serde_json::json!({"kind": "input"})
        }),
        Ok(Reply::Rejected { reason }) if reason == WORLD_CLOSED
    ));
    assert_eq!(second.whole_journal().unwrap().len(), 2);
}

fn wait_until(done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out");
}

/// `id-bevy-presenter world --headless` in its own process.
struct WorldProcess {
    child: Child,
    addr: String,
}

impl WorldProcess {
    fn start(path: &Path) -> Self {
        let mut child = Command::new(PROGRAM)
            .args([
                "world",
                "--headless",
                "--listen",
                "127.0.0.1:0",
                "--journal",
            ])
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let addr = line
            .trim()
            .strip_prefix("listening on ")
            .unwrap_or_else(|| panic!("unexpected first line {line:?}"))
            .to_owned();
        Self { child, addr }
    }

    /// The slice 4 app, in its own process, joins and plays `script`.
    fn play(&self, script: &str) -> serde_json::Value {
        let output = Command::new(PROGRAM)
            .args(["script", script, "--join", &self.addr])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn kill(mut self) {
        self.child.kill().unwrap();
        let _ = self.child.wait();
    }
}

#[test]
fn a_killed_host_process_loses_no_acknowledged_position() {
    let path = journal_path("killed");
    let world = WorldProcess::start(&path);
    let report = world.play("RRU");
    assert_eq!(report["replay"], serde_json::json!({ "Ok": null }));
    let acknowledged = connect_tcp(world.addr.as_str()).unwrap();
    acknowledged.hello(&device_manifest(640, 480)).unwrap();
    let journal = acknowledged.whole_journal().unwrap();
    assert_eq!(journal.last().map(|b| b.position), Some(5));
    drop(acknowledged);

    // No clean shutdown: the process is killed outright.
    world.kill();
    assert_eq!(
        JournalStore::open(&path).unwrap().batches(),
        journal.as_slice()
    );

    // A new host process resumes the stored world where it stopped.
    let world = WorldProcess::start(&path);
    let report = world.play("D");
    assert_eq!(report["replay"], serde_json::json!({ "Ok": null }));
    let events = report["events"].as_array().unwrap();
    let reconciled = events
        .iter()
        .find(|event| event["event"] == "reconciled")
        .unwrap();
    assert_eq!(reconciled["position"], 6);
    assert_eq!(reconciled["at"], serde_json::json!([2.0, 0.0]));
    world.kill();
    assert_eq!(JournalStore::open(&path).unwrap().head(), Some(6));
}
