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
    /// directories, every world asleep. Exports are signed with
    /// `host.key` in the data root, created on first run.
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
    /// Pin a world always-live, or unpin it with `--off`: a serving
    /// process never closes a pinned world for idleness or to make room
    /// for another. Acts on the running host when one answers on
    /// `host.sock`; otherwise edits the host file, which `id serve
    /// --world-host` picks up on its next look.
    Pin {
        /// The data root.
        root: PathBuf,
        /// The world.
        world: String,
        /// Unpin instead.
        #[arg(long)]
        off: bool,
        /// Print the world's settings as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Set how an asleep world wakes: `connect` (a client's connection
    /// wakes it, the default) or `operator` (only `id host start`).
    /// Reaches the host file as `pin` does.
    WakeMode {
        /// The data root.
        root: PathBuf,
        /// The world.
        world: String,
        /// `connect` or `operator`.
        #[arg(value_parser = ["connect", "operator"])]
        mode: String,
        /// Print the world's settings as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Print the host's export signing key (64 hex digits), creating
    /// `host.key` in the data root, readable by its owner alone, when it
    /// is missing. Give this to whoever restores this host's exports.
    Key {
        /// The data root.
        root: PathBuf,
    },
    /// Export a live world on the running host to a new file: its journal,
    /// directory facts and active module as signed replication frames.
    /// The world keeps serving. Start an asleep world first.
    Export {
        /// The data root the host runs on.
        root: PathBuf,
        /// The world.
        world: String,
        /// The file to write; it must not exist.
        file: PathBuf,
        /// Print the export manifest as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Restore an export file as a new asleep world in a data root,
    /// verifying every frame against the signing key you pin. Goes
    /// through the running host when one answers; otherwise writes the
    /// world directory and the host file itself. A tampered, cut or
    /// wrongly signed file restores nothing.
    Import {
        /// The data root.
        root: PathBuf,
        /// The export file.
        file: PathBuf,
        /// The exporting host's key, as its `id host key` prints it.
        #[arg(long)]
        source: String,
        /// The actor recorded on the adoption.
        #[arg(long, default_value = "operator")]
        actor: String,
        /// Print the export manifest as JSON instead of a line.
        #[arg(long)]
        json: bool,
    },
    /// Show or set scheduled snapshots (#13739): how often a running
    /// host takes a signed export of each live world, and how many sets
    /// it keeps. With no `--world` this is the host-wide default; with
    /// one, that world's own policy and its sets on disk. Give `--every`
    /// and `--keep` to set a policy, `--off` to turn snapshots off,
    /// `--inherit` to drop a world's own policy. With none of these it
    /// only shows. Acts on the running host when one answers on
    /// `host.sock`; otherwise edits the host file.
    Snapshots {
        /// The data root.
        root: PathBuf,
        /// One world instead of the host-wide default.
        #[arg(long)]
        world: Option<String>,
        /// Time between sets, such as `90m`, `6h` or `1d` (at least 60s).
        #[arg(long, requires = "keep", conflicts_with_all = ["off", "inherit"])]
        every: Option<String>,
        /// Newest sets kept.
        #[arg(long, requires = "every")]
        keep: Option<u32>,
        /// Also prune sets older than this, such as `30d`; the newest set
        /// is always kept.
        #[arg(long, requires = "every")]
        max_age: Option<String>,
        /// Turn scheduled snapshots off (sets already taken stay).
        #[arg(long, conflicts_with = "inherit")]
        off: bool,
        /// Drop the world's own policy so it follows the host default.
        #[arg(long, requires = "world")]
        inherit: bool,
        /// Print the settings and sets as JSON instead of lines.
        #[arg(long)]
        json: bool,
    },
    /// Show one world's record without opening it: lifecycle state and
    /// every recorded transition, journal entries, version and torn
    /// tail, module count, storage bytes, and what the last restart
    /// replay found. Writes nothing.
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
