//! atlas-eval — the offline ranking-evaluation runner (#5741).
//!
//! Usage: `atlas-eval <eval.json> <dev|test>` or
//! `atlas-eval --precision-at-k <k> <eval.json> <dev|test>`;
//! `atlas-eval --help` prints the usage line and `--version` the version,
//! both exit 0.
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
//!
//! Exit status is the eval's verdict: 0 when every signal passes,
//! 1 when the report contains ceiling/scope or provenance
//! violations (#8627 — the report is still printed first, as the
//! evidence), 2 on usage errors.

use std::process::ExitCode;

use atlas_core::eval::{EvalData, MAX_PRECISION_K, Split, evaluate, evaluate_precision};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (path, split, precision_k) = match args.as_slice() {
        [_, flag] if flag == "--help" || flag == "-h" => {
            print_usage();
            return ExitCode::SUCCESS;
        }
        [_, flag] if flag == "--version" || flag == "-V" => {
            println!("atlas-eval {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        [_, path, split] => {
            let split = match split.as_str() {
                "dev" => Split::Dev,
                "test" => Split::Test,
                other => {
                    eprintln!("atlas-eval: unknown split {other:?} (want dev|test)");
                    return ExitCode::from(2);
                }
            };
            (path.clone(), split, None)
        }
        [_, mode, k, path, split] if mode == "--precision-at-k" => {
            let k = match k.parse::<usize>() {
                Ok(k) => k,
                Err(_) => {
                    eprintln!(
                        "atlas-eval: invalid precision depth {k:?} (want an integer from 1 to 1000)"
                    );
                    return ExitCode::from(2);
                }
            };
            if !(1..=MAX_PRECISION_K).contains(&k) {
                eprintln!("atlas-eval: precision depth must be from 1 to {MAX_PRECISION_K}");
                return ExitCode::from(2);
            }
            let split = match split.as_str() {
                "dev" => Split::Dev,
                "test" => Split::Test,
                other => {
                    eprintln!("atlas-eval: unknown split {other:?} (want dev|test)");
                    return ExitCode::from(2);
                }
            };
            (path.clone(), split, Some(k))
        }
        _ => {
            eprintln!("{}", usage());
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
    let result = match precision_k {
        Some(k) => (|| {
            let precision = evaluate_precision(&data, split, k)?;
            let passed = evaluate(&data, split)?.passed();
            Ok::<_, atlas_core::error::Error>((
                serde_json::to_string_pretty(&precision).expect("report serializes"),
                passed,
            ))
        })(),
        None => evaluate(&data, split).map(|report| {
            (
                serde_json::to_string_pretty(&report).expect("report serializes"),
                report.passed(),
            )
        }),
    };
    match result {
        Ok((report, passed)) => {
            println!("{report}");
            if passed {
                return ExitCode::SUCCESS;
            }
            let report = evaluate(&data, split).expect("standard report computed above");
            for signal in &report.signals {
                if let Some(first) = signal.violations.first() {
                    eprintln!(
                        "atlas-eval: FAIL signal {:?}: {} constraint violation(s); \
                         first: query {:?} doc {:?} at rank {} ({:?})",
                        signal.signal,
                        signal.violations.len(),
                        first.query,
                        first.doc,
                        first.rank,
                        first.kind,
                    );
                }
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("atlas-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> &'static str {
    "usage: atlas-eval <eval.json> <dev|test>\n       atlas-eval --precision-at-k <k> <eval.json> <dev|test>"
}

fn print_usage() {
    println!("{}", usage());
}
