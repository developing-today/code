//! atlas-sweep — the Atlas ingestion sweep runner CLI (#7969).
//!
//! Usage: `atlas-sweep --state-dir DIR <action> ...`
//! (`--help` prints the usage text, `--version` the version;
//! both exit 0.)
//!
//! Actions (design §6, the complete operator set this core exposes):
//!   pause-class --class CLASS --host HOST
//!   resume-class --class CLASS --host HOST
//!   run-once REQUEST.json        (an atlas_core::sweep::RunRequest)
//!   dead-letter list|retry REF|drop REF
//!   report [--run RUN_ID]        (cycle report, or one run's row)
//!
//! The runner creates and enables NO schedule: cadence is an
//! operator/platform decision gated on Drew's go-ahead on #7969.
//! `run-once` prints its manifest records and report row as JSON;
//! an overlap with a live run prints `overlap-skip` and exits 0,
//! exactly as the §6 runbook prescribes.

use std::path::PathBuf;
use std::process::ExitCode;

use atlas_core::sweep::{
    RunRequest, SourceClass, cycle_report, dead_letter_drop, dead_letter_list, dead_letter_retry,
    pause_class, resume_class, run_once,
};

/// Execution failure (family contract, #15766): exit 1.
fn fail(msg: &str) -> ExitCode {
    eprintln!("atlas-sweep: {msg}");
    ExitCode::FAILURE
}

/// Usage / invalid-invocation error (family contract, #15766): exit 2.
fn usage_fail(msg: &str) -> ExitCode {
    eprintln!("atlas-sweep: {msg}");
    ExitCode::from(2)
}

const USAGE: &str = "usage: atlas-sweep --state-dir DIR <action> ...\n  \
    actions: pause-class --class CLASS --host HOST | resume-class --class CLASS --host HOST | \
    run-once REQUEST.json | dead-letter list|retry REF|drop REF | report [--run RUN_ID]";

fn class_and_host(args: &[String]) -> Result<(SourceClass, String), String> {
    let mut class = None;
    let mut host = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--class" => {
                class = args.get(i + 1).cloned();
                i += 2;
            }
            "--host" => {
                host = args.get(i + 1).cloned();
                i += 2;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let class = class.ok_or("missing --class")?;
    let class =
        SourceClass::from_name(&class).ok_or_else(|| format!("unknown source class {class:?}"))?;
    let host = host.ok_or("missing --host")?;
    Ok((class, host))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!(
            "atlas-sweep {}",
            atlas_core::version::version_string(env!("CARGO_PKG_VERSION"))
        );
        return ExitCode::SUCCESS;
    }
    let mut state_dir: Option<PathBuf> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--state-dir" {
            state_dir = args.get(i + 1).map(PathBuf::from);
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    let Some(state_dir) = state_dir else {
        return usage_fail("missing --state-dir DIR");
    };
    let Some((action, action_args)) = rest.split_first() else {
        return usage_fail("missing action (pause-class|resume-class|run-once|dead-letter|report)");
    };
    let result = match action.as_str() {
        "pause-class" | "resume-class" => class_and_host(action_args)
            .map_err(|e| format!("usage: {e}"))
            .and_then(|(class, host)| {
            let f = if action == "pause-class" {
                pause_class
            } else {
                resume_class
            };
            f(&state_dir, class, &host)
                .map(|()| {
                    format!(
                        "{} {}",
                        action,
                        atlas_core::sweep::class_host_key(class, &host)
                    )
                })
                .map_err(|e| e.to_string())
        }),
        "run-once" => {
            let Some(path) = action_args.first() else {
                return usage_fail("run-once needs a REQUEST.json path");
            };
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(e) => return fail(&format!("read {path}: {e}")),
            };
            let req: RunRequest = match serde_json::from_str(&text) {
                Ok(req) => req,
                Err(e) => return fail(&format!("parse {path}: {e}")),
            };
            match run_once(&state_dir, &req) {
                Ok(outcome) => {
                    if outcome.status == atlas_core::sweep::RunStatus::OverlapSkip {
                        println!("overlap-skip");
                        return ExitCode::SUCCESS;
                    }
                    let doc = serde_json::json!({
                        "status": format!("{:?}", outcome.status),
                        "records": outcome.records,
                        "report": outcome.report,
                        "alerts": outcome.alerts,
                    });
                    println!("{doc}");
                    Ok(String::new())
                }
                Err(e) => Err(e.to_string()),
            }
            .map(|_| String::new())
        }
        "dead-letter" => match action_args.first().map(String::as_str) {
            Some("list") => dead_letter_list(&state_dir)
                .map(|refs| refs.join("\n"))
                .map_err(|e| e.to_string()),
            Some("retry") => action_args
                .get(1)
                .ok_or_else(|| "usage: dead-letter retry needs a REF".to_string())
                .and_then(|r| dead_letter_retry(&state_dir, r).map_err(|e| e.to_string()))
                .map(|()| String::new()),
            Some("drop") => action_args
                .get(1)
                .ok_or_else(|| "usage: dead-letter drop needs a REF".to_string())
                .and_then(|r| dead_letter_drop(&state_dir, r).map_err(|e| e.to_string()))
                .map(|()| String::new()),
            _ => Err("usage: dead-letter needs list|retry REF|drop REF".to_string()),
        },
        "report" => {
            let rows = match cycle_report(&state_dir) {
                Ok(rows) => rows,
                Err(e) => return fail(&e.to_string()),
            };
            if let Some(pos) = action_args.iter().position(|a| a == "--run") {
                let Some(run_id) = action_args.get(pos + 1) else {
                    return usage_fail("report --run needs a RUN_ID");
                };
                match rows.iter().find(|row| &row.run_id == run_id) {
                    Some(row) => println!("{}", serde_json::json!(row)),
                    None => return fail(&format!("no report row for run {run_id}")),
                }
            } else {
                println!("{}", serde_json::json!(rows));
            }
            Ok(String::new())
        }
        other => Err(format!("usage: unknown action {other:?}")),
    };
    match result {
        Ok(text) => {
            if !text.is_empty() {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(msg) => match msg.strip_prefix("usage: ") {
            Some(msg) => usage_fail(msg),
            None => fail(&msg),
        },
    }
}
