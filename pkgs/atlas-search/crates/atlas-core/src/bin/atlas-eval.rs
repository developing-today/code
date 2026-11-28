//! atlas-eval — the offline ranking-evaluation runner (#5741).
//!
//! Usage: `atlas-eval <eval.json> <dev|test>`
//!
//! Reads an `atlas-eval/v1` document (queries, qrels, and saved
//! rankings from any signals) and prints the `atlas-eval-report/v1`
//! JSON report for the requested split to stdout. Fully offline and
//! deterministic: no model, no database, no network. See
//! `atlas_core::eval` for the format and metric definitions, and
//! `pkgs/atlas-search/eval/README.md` for the evaluation protocol.
//!
//! The runner only *scores* a split. Parameter selection on the
//! held-out test split is refused by `atlas_core::eval::
//! select_best_signal`; this binary prints test reports for the
//! final, single scoring pass the #5741 protocol allows.

use std::process::ExitCode;

use atlas_core::eval::{EvalData, Split, evaluate};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (path, split) = match args.as_slice() {
        [_, path, split] => {
            let split = match split.as_str() {
                "dev" => Split::Dev,
                "test" => Split::Test,
                other => {
                    eprintln!("atlas-eval: unknown split {other:?} (want dev|test)");
                    return ExitCode::from(2);
                }
            };
            (path.clone(), split)
        }
        _ => {
            eprintln!("usage: atlas-eval <eval.json> <dev|test>");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("atlas-eval: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let data = match EvalData::from_json(&text) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("atlas-eval: {error}");
            return ExitCode::FAILURE;
        }
    };
    match evaluate(&data, split) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("report serializes")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("atlas-eval: {error}");
            ExitCode::FAILURE
        }
    }
}
