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

use std::io::{self, BufReader};
use std::process::ExitCode;
use std::sync::Arc;

use bevy::prelude::{App, Camera2d, Commands, DefaultPlugins, Startup};
use id_bevy_presenter::host::ArenaHost;
use id_bevy_presenter::remote::{serve, spawn_host};
use id_bevy_presenter::{PresenterPlugin, join, parse_script, run_script};

fn grant_names(args: &[String]) -> Vec<String> {
    args.iter()
        .position(|arg| arg == "--grant")
        .and_then(|at| args.get(at + 1))
        .map_or_else(ArenaHost::every_surface, |names| {
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
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("id-bevy-presenter script: cannot find this program: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (mut child, link) = match spawn_host(&program, &host_args(args)) {
        Ok(spawned) => spawned,
        Err(error) => {
            eprintln!("id-bevy-presenter script: cannot start the host: {error}");
            return ExitCode::FAILURE;
        }
    };
    let link = Arc::new(link);
    let outcome = run_script(&link, &presses);
    drop(link);
    let _ = child.wait();
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
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("id-bevy-presenter: cannot find this program: {error}");
            return ExitCode::FAILURE;
        }
    };
    let (mut child, link) = match spawn_host(&program, &host_args(args)) {
        Ok(spawned) => spawned,
        Err(error) => {
            eprintln!("id-bevy-presenter: cannot start the host: {error}");
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
            "id-bevy-presenter: joined the demo world; granted surfaces: {}",
            grants.labels().join(", ")
        ),
        Err(error) => {
            eprintln!("id-bevy-presenter: cannot join: {error}");
            let _ = child.kill();
            return ExitCode::FAILURE;
        }
    }
    let exit = app.run();
    drop(app);
    drop(link);
    let _ = child.wait();
    if exit.is_error() {
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
        _ => run_window(&args),
    }
}
