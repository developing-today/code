//! atlas-stage — provision Atlas staging units and read the staging
//! registry (developing-today/code#5669, slice 1: the staging tier's
//! versioned schema, provisioning tool and registry; design
//! `doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision` section 4.1).
//!
//! Two verbs:
//! * `atlas-stage provision --db <path> --unit <id> --owner <o>
//!   [--sensitivity internal] --registry <path>` — create the unit's
//!   staging DB, apply the v1 schema, and register the unit as `open`.
//! * `atlas-stage lookup --unit <id> --registry <path>` — print the
//!   registry record as JSON.
//! * `atlas-stage units --state open --registry <path>` — list units
//!   in a lifecycle state (what the aggregator's poll reads).
//!
//! All three are offline: a staging DB is a local libSQL/SQLite file
//! today; hosted Turso creation is the aggregator slice's deployment
//! surface, not this foundation.

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::shards::Tier;
use atlas_core::stage::{Registry, UnitState};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "atlas-stage",
    about = "Provision Atlas staging units and read the staging registry (#5669)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a staging DB, apply the v1 schema, and register the unit.
    Provision {
        /// The staging database file to create.
        #[arg(long)]
        db: PathBuf,
        /// The staging-unit id (queue task id, session id, epic id, ...).
        #[arg(long)]
        unit: String,
        /// The unit's owner (the writer whose token may write here).
        #[arg(long)]
        owner: String,
        /// Physical sensitivity level (public|internal|secret; default internal).
        #[arg(long, default_value = "internal")]
        sensitivity: String,
        /// The registry store file (created if missing).
        #[arg(long)]
        registry: PathBuf,
    },
    /// Print one unit's registry record as JSON.
    Lookup {
        #[arg(long)]
        unit: String,
        #[arg(long)]
        registry: PathBuf,
    },
    /// List units in a lifecycle state (open|closing|merged).
    Units {
        #[arg(long, default_value = "open")]
        state: String,
        #[arg(long)]
        registry: PathBuf,
    },
}

fn now_utc() -> String {
    // RFC 3339, second precision: the registry records when, not
    // sub-second detail. Derived from the system clock without pulling
    // a datetime crate into atlas-core.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil-from-days (Howard Hinnant's algorithm), days since 1970-01-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn parse_tier(s: &str) -> Result<Tier, String> {
    Tier::parse(s).ok_or_else(|| format!("unknown sensitivity {s:?} (want public|internal|secret)"))
}

fn parse_state(s: &str) -> Result<UnitState, String> {
    UnitState::parse(s).ok_or_else(|| format!("unknown state {s:?} (want open|closing|merged)"))
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Provision {
            db,
            unit,
            owner,
            sensitivity,
            registry,
        } => {
            let sensitivity = parse_tier(&sensitivity)?;
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            let record = registry
                .provision(&db, &unit, &owner, sensitivity, &now_utc())
                .map_err(|e| e.to_string())?;
            println!("{}", unit_json(&record));
            Ok(())
        }
        Command::Lookup { unit, registry } => {
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            match registry.lookup(&unit).map_err(|e| e.to_string())? {
                Some(record) => {
                    println!("{}", unit_json(&record));
                    Ok(())
                }
                None => Err(format!("no staging unit {unit:?} in the registry")),
            }
        }
        Command::Units { state, registry } => {
            let state = parse_state(&state)?;
            let registry = Registry::open(&registry).map_err(|e| e.to_string())?;
            let units = registry.units_in_state(state).map_err(|e| e.to_string())?;
            for u in &units {
                println!("{}", unit_json(u));
            }
            Ok(())
        }
    }
}

fn unit_json(unit: &atlas_core::stage::StagingUnit) -> String {
    format!(
        "{{\"unit_id\":{},\"db_uri\":{},\"owner\":{},\"sensitivity\":{},\"state\":{},\"schema_version\":{},\"created_at\":{},\"updated_at\":{}}}",
        json_str(&unit.unit_id),
        json_str(&unit.db_uri),
        json_str(&unit.owner),
        json_str(unit.sensitivity.name()),
        json_str(unit.state.name()),
        unit.schema_version,
        json_str(&unit.created_at),
        json_str(&unit.updated_at),
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atlas-stage: {e}");
            ExitCode::FAILURE
        }
    }
}
