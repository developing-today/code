//! `id host` subcommand definitions: host mode over a data root of
//! world directories (#10693, packet P1 of the hosted-worlds v1 design
//! `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`).

use std::path::PathBuf;

use clap::Subcommand;

/// Subcommands for `id host`.
///
/// A data root holds one directory per world (`journal.jsonl` +
/// `modules/`, the same layout a laptop host uses) and the host file
/// `host.json` recording each world's lifecycle state: live, asleep
/// or archived.
#[derive(Subcommand, Debug)]
pub enum HostCommand {
    /// Run host mode over a data root until Ctrl-C.
    ///
    /// Replays every world the host file records live, then starts
    /// each `--start` world. While it runs, `id host start`, `stop`,
    /// `archive` and `restore` reach it over `host.sock` in the data
    /// root, a socket only the operator who runs the host can use.
    /// Ctrl-C drains every live world and leaves the host file as it
    /// stands, so the next run replays the same worlds back to live. A
    /// data root without a host file gets one rebuilt from its
    /// directories, every world asleep.
    Run {
        /// The data root.
        root: PathBuf,
        /// An asleep world to start once the recorded-live worlds have
        /// replayed; repeat to start several.
        #[arg(long = "start", value_name = "WORLD")]
        start: Vec<String>,
        /// The actor recorded on the transitions this run makes.
        #[arg(long, default_value = "operator")]
        actor: String,
    },
    /// List every world under a data root with its lifecycle state,
    /// storage bytes and last state change, without opening any world
    /// and without writing the host file.
    List {
        /// The data root.
        root: PathBuf,
        /// Print the census as JSON instead of lines.
        #[arg(long)]
        json: bool,
    },
    /// Start an asleep world on the running host: asleep to live, by
    /// replay. An archived world is refused until it is restored.
    Start {
        /// The data root the host runs on.
        root: PathBuf,
        /// The world.
        world: String,
        /// The actor recorded on the transition.
        #[arg(long, default_value = "operator")]
        actor: String,
        /// Print the host's answer as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Stop a live world on the running host: drain it, then record it
    /// asleep.
    Stop {
        /// The data root the host runs on.
        root: PathBuf,
        /// The world.
        world: String,
        /// The actor recorded on the transition.
        #[arg(long, default_value = "operator")]
        actor: String,
        /// Print the host's answer as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Archive an asleep world on the running host: it no longer wakes
    /// on its own, and an operator must restore it first.
    Archive {
        /// The data root the host runs on.
        root: PathBuf,
        /// The world.
        world: String,
        /// The actor recorded on the transition.
        #[arg(long, default_value = "operator")]
        actor: String,
        /// Print the host's answer as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Restore an archived world on the running host to asleep.
    Restore {
        /// The data root the host runs on.
        root: PathBuf,
        /// The world.
        world: String,
        /// The actor recorded on the transition.
        #[arg(long, default_value = "operator")]
        actor: String,
        /// Print the host's answer as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Show one world's record without opening it: lifecycle state and
    /// every recorded transition, journal entries, version and torn
    /// tail, module count and storage bytes. Writes nothing.
    Inspect {
        /// The data root.
        root: PathBuf,
        /// The world to inspect.
        world: String,
        /// Print the inspection as JSON instead of lines.
        #[arg(long)]
        json: bool,
    },
}
