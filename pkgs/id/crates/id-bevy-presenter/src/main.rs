//! `id-bevy-presenter`: the Bevy presenter-client reference app (#13705).
//!
//! - `id-bevy-presenter` opens a window, starts the demo world in a
//!   separate host process, joins it, and moves the avatar with the
//!   arrow keys, WASD or a gamepad's d-pad.
//! - `id-bevy-presenter host [--grant NAMES]` serves the demo world on
//!   standard input and output. `NAMES` is a comma-separated list of the
//!   provider names the world grants (default: every presenter surface).
//! - `id-bevy-presenter script MOVES [--grant NAMES]` runs headless
//!   against a host process, presses `MOVES` (`RLUD` for keys, `rlud` for
//!   the d-pad) and prints what the app showed as JSON. It exits non-zero
//!   if the replay oracle finds a divergence.
//! - `id-bevy-presenter world --journal PATH [--listen ADDR] [--grant
//!   NAMES] [--capacity N] [--headless]` hosts the demo world in this
//!   process (the host posture, #13706): its journal is stored at `PATH`
//!   (created with the arena if missing, resumed if present), its gate
//!   runs here, the window presents it through the same bridge path as
//!   any presenter, and other presenters join on `ADDR` (default
//!   `127.0.0.1:0`). It prints `listening on ADDR` once it is ready.
//!   `--headless` runs without a window.
//! - `--join ADDR` makes the window and `script` modes join a world
//!   served on `ADDR` instead of starting their own host process.

use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ExitCode};
use std::sync::Arc;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::{
    App, Camera2d, Commands, DefaultPlugins, MinimalPlugins, PluginGroup, Startup,
};
use id_bevy_presenter::host::ArenaHost;
use id_bevy_presenter::remote::{serve, spawn_host};
use id_bevy_presenter::world_host::DEFAULT_CAPACITY;
use id_bevy_presenter::{
    EmbeddedHost, HostSettings, PresenterPlugin, RemoteWorld, connect_tcp, join, listen,
    parse_script, present_hosted, run_script,
};

/// The headless host's tick interval.
const HEADLESS_TICK: Duration = Duration::from_millis(16);

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .map(String::as_str)
}

/// The world to join: the one on `--join ADDR`, or a new host process.
/// The child, if any, must be waited for once the link is dropped.
fn link_to_world(args: &[String]) -> io::Result<(Option<Child>, RemoteWorld)> {
    if let Some(addr) = flag(args, "--join") {
        return Ok((None, connect_tcp(addr)?));
    }
    let program = std::env::current_exe()?;
    let (child, link) = spawn_host(&program, &host_args(args))?;
    Ok((Some(child), link))
}

fn grant_names(args: &[String]) -> Vec<String> {
    flag(args, "--grant").map_or_else(ArenaHost::every_surface, |names| {
        names
            .split(',')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn host_args(args: &[String]) -> Vec<String> {
    let mut forwarded = vec!["host".to_owned()];
    if let Some(at) = args.iter().position(|arg| arg == "--grant") {
        forwarded.extend(args.iter().skip(at).take(2).cloned());
    }
    forwarded
}

fn run_host(args: &[String]) -> ExitCode {
    let world = ArenaHost::demo(grant_names(args));
    let mut reader = BufReader::new(io::stdin().lock());
    let mut writer = io::stdout().lock();
    match serve(&world, &mut reader, &mut writer) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("id-bevy-presenter host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_scripted(args: &[String]) -> ExitCode {
    let Some(moves) = args.get(1) else {
        eprintln!("usage: id-bevy-presenter script MOVES [--grant NAMES]");
        return ExitCode::from(2);
    };
    let presses = match parse_script(moves) {
        Ok(presses) => presses,
        Err(step) => {
            eprintln!("id-bevy-presenter script: {step:?} is not a move (use RLUD or rlud)");
            return ExitCode::from(2);
        }
    };
    let (child, link) = match link_to_world(args) {
        Ok(linked) => linked,
        Err(error) => {
            eprintln!("id-bevy-presenter script: cannot reach the world: {error}");
            return ExitCode::FAILURE;
        }
    };
    let link = Arc::new(link);
    let outcome = run_script(&link, &presses);
    drop(link);
    if let Some(mut child) = child {
        let _ = child.wait();
    }
    match outcome {
        Ok(run) => {
            println!("{}", serde_json::to_string_pretty(&run).unwrap_or_default());
            if run.replay.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("id-bevy-presenter script: {error}");
            ExitCode::FAILURE
        }
    }
}

fn spawn_camera(mut commands: Commands<'_, '_>) {
    commands.spawn(Camera2d);
}

fn run_window(args: &[String]) -> ExitCode {
    let (mut child, link) = match link_to_world(args) {
        Ok(linked) => linked,
        Err(error) => {
            eprintln!("id-bevy-presenter: cannot reach the world: {error}");
            return ExitCode::FAILURE;
        }
    };
    let link = Arc::new(link);
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(PresenterPlugin)
        .add_systems(Startup, spawn_camera);
    match join(&mut app, &link) {
        Ok(grants) => println!(
            "id-bevy-presenter: joined the world; granted surfaces: {}",
            grants.labels().join(", ")
        ),
        Err(error) => {
            eprintln!("id-bevy-presenter: cannot join: {error}");
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
            return ExitCode::FAILURE;
        }
    }
    let exit = app.run();
    drop(app);
    drop(link);
    if let Some(mut child) = child {
        let _ = child.wait();
    }
    if exit.is_error() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_world(args: &[String]) -> ExitCode {
    let Some(path) = flag(args, "--journal").map(PathBuf::from) else {
        eprintln!(
            "usage: id-bevy-presenter world --journal PATH [--listen ADDR] [--grant NAMES] [--capacity N] [--headless]"
        );
        return ExitCode::from(2);
    };
    let capacity = match flag(args, "--capacity").map(str::parse::<usize>) {
        None => DEFAULT_CAPACITY,
        Some(Ok(capacity)) => capacity,
        Some(Err(error)) => {
            eprintln!("id-bevy-presenter world: --capacity: {error}");
            return ExitCode::from(2);
        }
    };
    let mut settings = HostSettings::arena(grant_names(args));
    settings.capacity = capacity;
    let opened = if path.exists() {
        EmbeddedHost::open(&path, settings)
    } else {
        EmbeddedHost::create(&path, id_bevy_presenter::host::arena_genesis(), settings)
    };
    let host = match opened {
        Ok(host) => Arc::new(host),
        Err(error) => {
            eprintln!("id-bevy-presenter world: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App::new();
    if args.iter().any(|arg| arg == "--headless") {
        app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(HEADLESS_TICK)));
    } else {
        app.add_plugins(DefaultPlugins)
            .add_systems(Startup, spawn_camera);
    }
    app.add_plugins(PresenterPlugin);
    if let Err(error) = present_hosted(&mut app, &host) {
        eprintln!("id-bevy-presenter world: {error}");
        return ExitCode::FAILURE;
    }
    let addr = flag(args, "--listen").unwrap_or("127.0.0.1:0");
    let mut door = match listen(Arc::clone(&host), addr) {
        Ok(door) => door,
        Err(error) => {
            eprintln!("id-bevy-presenter world: cannot listen on {addr}: {error}");
            return ExitCode::FAILURE;
        }
    };
    println!("listening on {}", door.addr());
    let _ = io::stdout().flush();
    let exit = app.run();
    door.stop();
    let closed = host.close();
    if let Err(error) = &closed {
        eprintln!("id-bevy-presenter world: {error}");
    }
    if exit.is_error() || closed.is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("host") => run_host(&args),
        Some("script") => run_scripted(&args),
        Some("world") => run_world(&args),
        _ => run_window(&args),
    }
}
