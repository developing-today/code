//! #12406 LIVE INVENTORY (manual; not a CI test) — SELECT-only inventory of
//! the monolith's secret-bearing chunks against the secret-tier destination.
//!
//! Reads ONLY id, content_hash, sensitivity (+ vector presence). No chunk
//! text is ever selected. Writes NOTHING to any shard; the JSON report goes
//! to a local file. Trace-safe: identifiers, hashes and counts only.
//!
//! The complete CLI is `remediation_inventory --out PATH [--resume PATH]
//! [--vec-table IDENTIFIER]`. Repeated, unknown, missing, or empty arguments
//! are rejected before any network request or file write.
//!
//! Run on an operator host with explicit endpoint hosts:
//! ```sh
//! MONOLITH_HOST=... SECRET_HOST=... MONOLITH_TOKEN=... SECRET_TOKEN=... \
//!   cargo run -p atlas-core --example remediation_inventory --release \
//!     -- --out /path/to/new-report.json [--resume /path/to/prior-report.json] \
//!     [--vec-table TABLE]
//! ```
use std::path::{Path, PathBuf};

use atlas_core::filters::Sensitivity;
use atlas_core::remediation::{
    inventory, is_valid_vector_table_identifier, read_run, validate_resume, write_run,
};
use atlas_core::shards::LocalShardTransport;
use atlas_core::turso::HttpTransport;

fn env(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("{name} is not set or is empty"))
}

struct Args {
    out: PathBuf,
    resume: Option<PathBuf>,
    vec_table: String,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut out = None;
    let mut resume = None;
    let mut vec_table = None;
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{flag} requires a non-empty value"))?;
        match flag.as_str() {
            "--out" if out.is_none() => out = Some(PathBuf::from(value)),
            "--resume" if resume.is_none() => resume = Some(PathBuf::from(value)),
            "--vec-table" if vec_table.is_none() => vec_table = Some(value),
            "--out" | "--resume" | "--vec-table" => {
                return Err(format!("{flag} may be specified only once"));
            }
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    let vec_table = vec_table.unwrap_or_else(|| atlas_core::types::DEFAULT_VEC_TABLE.to_string());
    if !is_valid_vector_table_identifier(&vec_table) {
        return Err("--vec-table must be a plain SQL identifier".to_string());
    }
    Ok(Args {
        out: out.ok_or_else(|| "--out <path> is required".to_string())?,
        resume,
        vec_table,
    })
}

fn ensure_new_output(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "output already exists; choose a new path: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot inspect output path {}: {error}",
            path.display()
        )),
    }
}

fn main() {
    let args = parse_args(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("usage: remediation_inventory --out PATH [--resume PATH] [--vec-table IDENTIFIER]: {error}");
        std::process::exit(2);
    });
    ensure_new_output(&args.out).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let previous = args
        .resume
        .as_ref()
        .map(|path| read_run(path))
        .transpose()
        .unwrap_or_else(|error| {
            eprintln!("invalid prior run: {error}");
            std::process::exit(2);
        });
    // Local replica source: MONOLITH_FILE=<path> selects the read-only
    // LocalShardTransport; otherwise the hosted monolith is used.
    if let Ok(path) = std::env::var("MONOLITH_FILE") {
        let canonical_path = std::fs::canonicalize(&path).expect("canonicalize local replica");
        let monolith = LocalShardTransport::open(&canonical_path).expect("open local replica");
        let secret_host = env("SECRET_HOST");
        let secret = HttpTransport::new(&secret_host, env("SECRET_TOKEN"));
        let report = inventory(
            &monolith,
            &secret,
            &format!("file://{}", canonical_path.display()),
            &format!("turso://{secret_host}/atlas-core-secret-main-01"),
            Sensitivity::SecretBearing,
            &args.vec_table,
        )
        .expect("inventory failed");
        if let Some(previous) = &previous {
            validate_resume(previous, &report).expect("resume drift check");
        }
        write_run(&args.out, &report).expect("write report failed");
        println!(
            "source={} destination={} verified={} missing_destination={} hash_mismatch={} missing_vector={} destination_complete={} run_id={}",
            report.counts.source,
            report.counts.destination,
            report.counts.verified,
            report.counts.missing_destination,
            report.counts.hash_mismatch,
            report.counts.missing_vector,
            report.destination_complete(),
            report.run_id,
        );
        println!("report: {}", args.out.display());
        return;
    }
    let monolith_host = env("MONOLITH_HOST");
    let secret_host = env("SECRET_HOST");
    let monolith = HttpTransport::new(&monolith_host, env("MONOLITH_TOKEN"));
    let secret = HttpTransport::new(&secret_host, env("SECRET_TOKEN"));
    let report = inventory(
        &monolith,
        &secret,
        &format!("turso://{monolith_host}/atlas"),
        &format!("turso://{secret_host}/atlas-core-secret-main-01"),
        Sensitivity::SecretBearing,
        &args.vec_table,
    )
    .expect("inventory failed");
    if let Some(previous) = &previous {
        validate_resume(previous, &report).expect("resume drift check");
    }
    write_run(&args.out, &report).expect("write report failed");
    println!(
        "source={} destination={} verified={} missing_destination={} hash_mismatch={} missing_vector={} destination_complete={} run_id={}",
        report.counts.source,
        report.counts.destination,
        report.counts.verified,
        report.counts.missing_destination,
        report.counts.hash_mismatch,
        report.counts.missing_vector,
        report.destination_complete(),
        report.run_id,
    );
    println!("report: {}", args.out.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Result<Args, String> {
        parse_args(values.iter().map(|value| (*value).to_string()))
    }

    #[test]
    fn cli_contract_accepts_only_unique_declared_arguments() {
        let parsed = args(&[
            "--out",
            "report.json",
            "--resume",
            "prior.json",
            "--vec-table",
            "vectors",
        ])
        .expect("declared arguments");
        assert_eq!(parsed.out, PathBuf::from("report.json"));
        assert_eq!(parsed.resume, Some(PathBuf::from("prior.json")));
        assert_eq!(parsed.vec_table, "vectors");
        assert!(args(&["--out", "report.json", "--out", "other.json"]).is_err());
        assert!(args(&["--out", "report.json", "--unknown", "value"]).is_err());
        assert!(args(&["--out", "--resume"]).is_err());
        assert!(
            args(&[
                "--out",
                "report.json",
                "--vec-table",
                "vectors;DROP TABLE chunks"
            ])
            .is_err()
        );
    }

    #[test]
    fn output_guard_rejects_hard_link_alias_to_source() {
        let source = std::env::temp_dir().join(format!(
            "atlas-remediation-source-{}.db",
            std::process::id()
        ));
        let output = std::env::temp_dir().join(format!(
            "atlas-remediation-output-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        std::fs::write(&source, b"source shard bytes").expect("create source fixture");
        std::fs::hard_link(&source, &output).expect("link report path to source");

        assert!(ensure_new_output(&output).is_err());
        assert_eq!(
            std::fs::read(&source).expect("source remains"),
            b"source shard bytes"
        );

        let _ = std::fs::remove_file(source);
        let _ = std::fs::remove_file(output);
    }
}
