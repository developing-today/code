//! atlas-classify — the Atlas classification operating procedure
//! CLI (#12405): the unclassified-remainder sweep and the
//! schema-change review from the classification design
//! (`doc/2026-11-21T21-32-00Z_design_atlas_classification_model_v1.md`,
//! §5.1/§5.2), over the decision records of #12402.
//!
//! Command-line contract (#3352): the whole command line is
//! declared here and parsed strictly before any file is opened;
//! anything unaccounted for is a usage error with exit code 2.
//!
//! Usage:
//!   atlas-classify --db CORPUS.db run [--commit]
//!       [--window-days N] [--actor A] [--now TS]
//!       [--schema-version N] [--max-decisions N]
//!       [--manifest-out PATH]
//!   atlas-classify --db CORPUS.db schema-review
//!       --old-schema OLD.json --new-schema NEW.json
//!       [--reviewer-model M] [--reviewer-model-version V]
//!       [--manifest-out PATH]
//!   atlas-classify --db CORPUS.db schema-apply
//!       --review REVIEW.json --dispositions DISPOSITIONS.json
//!       [--now TS] [--manifest-out PATH]
//!   atlas-classify --help | --version
//!
//! Defaults: --window-days 90, --actor
//! `atlas-classify/remainder-sweep`, --schema-version 1,
//! --reviewer-model `atlas-classify`, --reviewer-model-version
//! the crate version, --now the current time, --max-decisions
//! unbounded.
//!
//! `run` is a dry-run unless `--commit` is given: it enumerates
//! only unclassified/default-state/unresolved embeds, prints the
//! run manifest (counts by prior state and outcome, the declared
//! relatedness window, vector-preservation hashes, and every
//! decision) as JSON, and writes nothing. With `--commit` it
//! writes one classification record per candidate into the
//! corpus's `classification_records` table; chunks and vectors
//! are never modified. Runs are deterministic and resumable:
//! re-running a committed run skips what is already decided.
//!
//! `schema-review` compares two schema documents section by
//! section (changed sections only, unless the new document
//! declares a breaking-change event) and prints the review
//! batch: proposals plus model/config/schema provenance. It
//! writes nothing. `schema-apply` applies a review batch's
//! proposals, each only with a reviewed disposition naming a
//! reviewer and rationale; lowerings without one are refused.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use atlas_core::classify::{self, Disposition, ReviewerIdentity, RunConfig, SchemaDoc, iso_now};
use serde_json::Value as Json;

fn fail(msg: &str) -> ExitCode {
    eprintln!("atlas-classify: {msg}");
    ExitCode::from(2)
}

const USAGE: &str = "usage:\n  atlas-classify --db CORPUS run [--commit] [--window-days N] [--actor A] \
         [--now TS] [--schema-version N] [--max-decisions N] [--manifest-out PATH]\n  \
         atlas-classify --db CORPUS schema-review --old-schema F --new-schema F \
         [--reviewer-model M] [--reviewer-model-version V] [--manifest-out PATH]\n  \
         atlas-classify --db CORPUS schema-apply --review F --dispositions F \
         [--now TS] [--manifest-out PATH]\n  \
         defaults: --window-days 90; --actor atlas-classify/remainder-sweep; \
         --schema-version 1; --reviewer-model atlas-classify; \
         --reviewer-model-version the crate version; --now the current time; \
         --max-decisions unbounded";

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// A strict flag set: every token must be a declared flag, with
/// its value where the declaration requires one.
struct Flags {
    values: BTreeMap<String, String>,
    switches: Vec<String>,
}

impl Flags {
    fn parse(
        args: &[String],
        value_flags: &[&str],
        switch_flags: &[&str],
    ) -> Result<Flags, String> {
        let mut values = BTreeMap::new();
        let mut switches = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if value_flags.contains(&arg.as_str()) {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{arg} needs a value"))?;
                if values.insert(arg.clone(), value.clone()).is_some() {
                    return Err(format!("{arg} given twice"));
                }
                i += 2;
            } else if switch_flags.contains(&arg.as_str()) {
                if switches.contains(arg) {
                    return Err(format!("{arg} given twice"));
                }
                switches.push(arg.clone());
                i += 1;
            } else {
                return Err(format!("unknown argument {arg:?}"));
            }
        }
        Ok(Flags { values, switches })
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn require(&self, name: &str) -> Result<&str, String> {
        self.get(name).ok_or_else(|| format!("missing {name}"))
    }

    fn has(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

fn now_default() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_now(secs)
}

fn read_json(path: &str) -> Result<Json, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))
}

fn emit(doc: &Json, manifest_out: Option<&str>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| format!("serialize manifest: {e}"))?;
    if let Some(path) = manifest_out {
        std::fs::write(path, format!("{text}\n")).map_err(|e| format!("write {path}: {e}"))?;
    }
    println!("{text}");
    Ok(())
}

fn parse_u64(raw: &str, flag: &str) -> Result<u64, String> {
    raw.parse::<u64>()
        .map_err(|_| format!("{flag} must be an unsigned integer, got {raw:?}"))
}

fn action_run(db: &Path, args: &[String]) -> Result<Json, String> {
    let flags = Flags::parse(
        args,
        &[
            "--window-days",
            "--actor",
            "--now",
            "--schema-version",
            "--max-decisions",
            "--manifest-out",
        ],
        &["--commit"],
    )?;
    let mut config = RunConfig {
        now: flags
            .get("--now")
            .map(str::to_owned)
            .unwrap_or_else(now_default),
        ..RunConfig::default()
    };
    if let Some(actor) = flags.get("--actor") {
        config.actor = actor.to_string();
    }
    if let Some(days) = flags.get("--window-days") {
        config.window_days = days
            .parse::<i64>()
            .map_err(|_| format!("--window-days must be an integer, got {days:?}"))?;
        if config.window_days < 0 {
            return Err("--window-days must not be negative".to_string());
        }
    }
    if let Some(v) = flags.get("--schema-version") {
        config.schema_version = parse_u64(v, "--schema-version")?;
    }
    if let Some(v) = flags.get("--max-decisions") {
        config.max_decisions = Some(
            parse_u64(v, "--max-decisions")?
                .try_into()
                .map_err(|_| "--max-decisions is too large".to_string())?,
        );
    }
    let manifest = classify::run_classification(db, flags.has("--commit"), &config)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&manifest).map_err(|e| format!("serialize manifest: {e}"))
}

fn action_schema_review(db: &Path, args: &[String]) -> Result<Json, String> {
    let flags = Flags::parse(
        args,
        &[
            "--old-schema",
            "--new-schema",
            "--reviewer-model",
            "--reviewer-model-version",
            "--manifest-out",
        ],
        &[],
    )?;
    let old =
        SchemaDoc::parse(&read_json(flags.require("--old-schema")?)?).map_err(|e| e.to_string())?;
    let new =
        SchemaDoc::parse(&read_json(flags.require("--new-schema")?)?).map_err(|e| e.to_string())?;
    // The review's config provenance binds the reviewer identity
    // and both schema documents.
    let config_canonical = format!(
        "atlas-classify-schema-review/1\nmodel={}\nmodel_version={}\nold={}\nnew={}\n",
        flags.get("--reviewer-model").unwrap_or("atlas-classify"),
        flags
            .get("--reviewer-model-version")
            .unwrap_or(env!("CARGO_PKG_VERSION")),
        old.content_hash(),
        new.content_hash(),
    );
    let reviewer = ReviewerIdentity {
        model: flags
            .get("--reviewer-model")
            .unwrap_or("atlas-classify")
            .to_string(),
        model_version: flags
            .get("--reviewer-model-version")
            .unwrap_or(env!("CARGO_PKG_VERSION"))
            .to_string(),
        config_version: format!(
            "sha256:{}",
            classify::sha256_hex(config_canonical.as_bytes())
        ),
    };
    let review =
        classify::review_schema_change(db, &old, &new, reviewer).map_err(|e| e.to_string())?;
    serde_json::to_value(&review).map_err(|e| format!("serialize review: {e}"))
}

fn action_schema_apply(db: &Path, args: &[String]) -> Result<Json, String> {
    let flags = Flags::parse(
        args,
        &["--review", "--dispositions", "--now", "--manifest-out"],
        &[],
    )?;
    let review: classify::SchemaReview =
        serde_json::from_value(read_json(flags.require("--review")?)?)
            .map_err(|e| format!("parse review: {e}"))?;
    let dispositions: BTreeMap<String, Disposition> =
        classify::parse_dispositions(&read_json(flags.require("--dispositions")?)?)
            .map_err(|e| e.to_string())?;
    let now = flags
        .get("--now")
        .map(str::to_owned)
        .unwrap_or_else(now_default);
    let report = classify::apply_schema_review(db, &review, &dispositions, &now)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(&report).map_err(|e| format!("serialize report: {e}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("atlas-classify {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    // Global flags: exactly one --db, then the action and its args.
    let mut db: Option<PathBuf> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--db" {
            if db.is_some() {
                return fail("--db given twice");
            }
            match args.get(i + 1) {
                Some(path) => {
                    db = Some(PathBuf::from(path));
                    i += 2;
                }
                None => return fail("--db needs a value"),
            }
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    let Some(db) = db else {
        return usage();
    };
    let Some((action, action_args)) = rest.split_first() else {
        return usage();
    };
    let result = match action.as_str() {
        "run" => action_run(&db, action_args),
        "schema-review" => action_schema_review(&db, action_args),
        "schema-apply" => action_schema_apply(&db, action_args),
        other => return fail(&format!("unknown action {other:?}")),
    };
    match result {
        Ok(doc) => {
            // Actions return the document; print it, and also
            // write it when the action carried --manifest-out.
            let manifest_out = action_args
                .iter()
                .position(|a| a == "--manifest-out")
                .and_then(|pos| action_args.get(pos + 1))
                .map(String::as_str);
            match emit(&doc, manifest_out) {
                Ok(()) => ExitCode::SUCCESS,
                Err(msg) => fail(&msg),
            }
        }
        Err(msg) => fail(&msg),
    }
}
