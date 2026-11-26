//! #12406 LIVE INVENTORY (manual; not a CI test) — SELECT-only inventory of
//! the monolith's secret-bearing chunks against the secret-tier destination.
//!
//! Reads ONLY id, content_hash, sensitivity (+ vector presence). No chunk
//! text is ever selected. Writes NOTHING to any shard; the JSON report goes
//! to a local file. Trace-safe: identifiers, hashes and counts only.
//!
//! Run on the laptop from the task worktree:
//! ```sh
//! SECRET_HOST=... MONOLITH_TOKEN=... SECRET_TOKEN=... \
//!   cargo run -p atlas-core --example remediation_inventory --release \
//!     -- --out /path/to/report.json [--vec-table TABLE]
//! ```
use std::path::PathBuf;

use atlas_core::filters::Sensitivity;
use atlas_core::remediation::{inventory, write_run};
use atlas_core::shards::LocalShardTransport;
use atlas_core::turso::HttpTransport;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn main() {
    let mut out: Option<PathBuf> = None;
    let mut vec_table = atlas_core::types::DEFAULT_VEC_TABLE.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = Some(PathBuf::from(args.next().expect("--out needs a path"))),
            "--vec-table" => vec_table = args.next().expect("--vec-table needs a value"),
            other => panic!("unknown arg {other}"),
        }
    }
    let out = out.expect("--out <report.json> is required");
    // Local replica source: MONOLITH_FILE=<path> selects the read-only
    // LocalShardTransport; otherwise the hosted monolith is used.
    if let Ok(path) = std::env::var("MONOLITH_FILE") {
        let monolith =
            LocalShardTransport::open(std::path::Path::new(&path)).expect("open local replica");
        let secret_host = std::env::var("SECRET_HOST")
            .unwrap_or_else(|_| panic!("SECRET_HOST is required with MONOLITH_FILE"));
        let secret = HttpTransport::new(&secret_host, env("SECRET_TOKEN"));
        let report = inventory(
            &monolith,
            &secret,
            &format!("replica:{path}"),
            "atlas-core-secret-main-01",
            Sensitivity::SecretBearing,
            &vec_table,
        )
        .expect("inventory failed");
        write_run(&out, &report).expect("write report failed");
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
        println!("report: {}", out.display());
        return;
    }
    let monolith_host = std::env::var("MONOLITH_HOST")
        .unwrap_or_else(|_| atlas_core::turso::TURSO_HOST.to_string());
    // Never infer the destination from the monolith hostname. A fabricated
    // hostname could connect the verifier to the wrong database.
    let secret_host = env("SECRET_HOST");
    let monolith = HttpTransport::new(&monolith_host, env("MONOLITH_TOKEN"));
    let secret = HttpTransport::new(&secret_host, env("SECRET_TOKEN"));
    let report = inventory(
        &monolith,
        &secret,
        "monolith",
        "atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &vec_table,
    )
    .expect("inventory failed");
    write_run(&out, &report).expect("write report failed");
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
    println!("report: {}", out.display());
}
