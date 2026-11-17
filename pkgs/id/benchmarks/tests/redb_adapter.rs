//! Transaction fixture for the redb workload adapter
//! (slice C9133-02-redb, issue #10164).
//!
//! Oracle: **reference** — the redb adapter must return the same
//! logical rows and generation transitions as the benchmark
//! reference model (C9133-01). This test executes the synthetic
//! workload fixture against both a real redb-backed store and the
//! reference store, step by step, and requires every observed
//! transition to equal the fixture's expected transition and the
//! reference store's transition. No timing threshold.
//!
//! Run: `cargo test --manifest-path pkgs/id/benchmarks/Cargo.toml --test redb_adapter`

#[path = "../src/adapters/redb.rs"]
mod redb_adapter;

use id_benchmarks::{content_id, support_level, ReferenceStore};
use redb_adapter::RedbStore;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn design_dir() -> PathBuf {
    crate_root()
        .join("../../../doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c")
        .canonicalize()
        .expect("design part-c dir exists")
}

fn read_json(path: &PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read json")).expect("parse json")
}

fn normalized(mut observed: Value, expected: &Value) -> Value {
    // Mirrors the reference oracle test: content_id is only part of
    // the observed shape when the fixture step expects it.
    if expected.get("content_id").is_none() {
        observed
            .as_object_mut()
            .expect("observed object")
            .remove("content_id");
    }
    observed
}

#[test]
fn redb_adapter_matches_reference_oracle() {
    let fixture_bytes = std::fs::read(crate_root().join("fixtures/synthetic-workload.json"))
        .expect("crate fixture");
    let fixture: Value = serde_json::from_slice(&fixture_bytes).expect("parse fixture");
    assert_eq!(fixture["format"], "id-atlas-synthetic-workload/v3");
    let plan = read_json(&design_dir().join("benchmark-plan.json"));
    let plan_ops: BTreeMap<String, Value> = plan["logical_operations"]
        .as_array()
        .expect("plan ops")
        .iter()
        .map(|row| (row["id"].as_str().expect("op id").to_string(), row.clone()))
        .collect();

    // Fixture digest, recomputed from the fixture's exact put bytes.
    let first = &fixture["operations"][0];
    let raw = hex::decode(first["input"]["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
    assert_eq!(content_id(&raw), fixture["expected_content_id"]);

    let mut reference = ReferenceStore::new();
    let mut redb = RedbStore::temp();
    assert!(redb.path().exists(), "temp redb database file exists");
    let mut executed = 0usize;
    for step in fixture["operations"].as_array().expect("operations") {
        let op_id = step["id"].as_str().expect("step id");
        let row = plan_ops
            .get(op_id)
            .unwrap_or_else(|| panic!("unmapped operation: {op_id}"));

        // The plan's redb cell must map to the fixture's recorded
        // redb support level for this operation.
        let cell = row["redb"].as_str().expect("redb plan cell");
        assert_eq!(
            Value::String(support_level(op_id, "redb", cell)),
            step["engine_support"]["redb"],
            "redb support mismatch for {op_id}"
        );

        let expected = &step["expected"];
        let reference_observed =
            normalized(reference.execute(op_id, &step["input"], &fixture), expected);
        let redb_observed = normalized(redb.execute(op_id, &step["input"], &fixture), expected);
        assert_eq!(
            redb_observed, *expected,
            "fixture transition mismatch for {op_id}"
        );
        assert_eq!(
            redb_observed, reference_observed,
            "redb adapter diverged from the reference model for {op_id}"
        );
        executed += 1;
    }
    assert_eq!(executed, 16, "redb transitions executed");
}

#[test]
fn redb_adapter_persists_across_reopen() {
    let fixture_bytes = std::fs::read(crate_root().join("fixtures/synthetic-workload.json"))
        .expect("crate fixture");
    let fixture: Value = serde_json::from_slice(&fixture_bytes).expect("parse fixture");
    let ops = fixture["operations"].as_array().expect("operations");

    let path = std::env::temp_dir().join(format!(
        "id-benchmarks-redb-reopen-{}.redb",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    {
        let mut store = RedbStore::open(&path);
        // Steps 0 (put), 5 (ref_commit_if_live) from the fixture.
        for idx in [0usize, 5] {
            let step = &ops[idx];
            let observed = normalized(
                store.execute(
                    step["id"].as_str().expect("step id"),
                    &step["input"],
                    &fixture,
                ),
                &step["expected"],
            );
            assert_eq!(observed, step["expected"], "step {idx} before reopen");
        }
    }
    {
        // A fresh open of the same database file must observe the
        // committed transactions: generation 1, Live, one reference.
        let mut store = RedbStore::open(&path);
        let get = store.execute("blob_get", &serde_json::json!({}), &fixture);
        assert_eq!(get, ops[2]["expected"], "blob_get after reopen");
        let lookup = store.execute("metadata_lookup", &serde_json::json!({}), &fixture);
        assert_eq!(lookup, ops[3]["expected"], "metadata_lookup after reopen");
        // Idempotent put reports the persisted reference count.
        let put = normalized(
            store.execute("blob_put_if_absent", &ops[0]["input"], &fixture),
            &ops[6]["expected"],
        );
        assert_eq!(
            put, ops[6]["expected"],
            "reference count persisted after reopen"
        );
    }
    let _ = std::fs::remove_file(&path);
}
