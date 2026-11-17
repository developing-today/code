//! Reference oracle for the #9133 benchmark contract (C9133-01):
//! executes the synthetic workload fixture against the finite
//! reference store in `id_benchmarks` and compares every transition
//! and every engine capability mapping row-by-row, exactly as the
//! design's Python oracle (benchmark-workload-oracle.py) does.
//! No timings, no provider calls.
//!
//! Oracle: the fixture + plan are the specification; the reference
//! store is the model under test. Digest is recomputed from the
//! fixture's exact bytes (SHA-256 of the put payload).

use id_benchmarks::{content_id, support_level, ReferenceStore, ENGINES};
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

#[test]
fn workload_oracle() {
    // The crate's fixture copy must be byte-identical to the design's
    // owned fixture, so the two can never drift apart silently.
    let fixture_bytes = std::fs::read(crate_root().join("fixtures/synthetic-workload.json"))
        .expect("crate fixture");
    let design_fixture_bytes =
        std::fs::read(design_dir().join("synthetic-workload.json")).expect("design fixture");
    assert_eq!(
        fixture_bytes, design_fixture_bytes,
        "fixture copy drifted from the design fixture"
    );

    let fixture: Value = serde_json::from_slice(&fixture_bytes).expect("parse fixture");
    let plan = read_json(&design_dir().join("benchmark-plan.json"));
    assert_eq!(fixture["format"], "id-atlas-synthetic-workload/v3");

    let plan_ops: BTreeMap<String, Value> = plan["logical_operations"]
        .as_array()
        .expect("plan ops")
        .iter()
        .map(|row| (row["id"].as_str().expect("op id").to_string(), row.clone()))
        .collect();
    for (op_id, row) in &plan_ops {
        for engine in ENGINES {
            let cell = row.get(engine).and_then(Value::as_str);
            assert!(
                matches!(cell, Some(s) if !s.is_empty()),
                "plan cell {op_id}/{engine}"
            );
        }
    }

    // Fixture digest, recomputed from the fixture's exact put bytes.
    let first = &fixture["operations"][0];
    let raw = hex::decode(first["input"]["bytes_hex"].as_str().expect("bytes_hex")).expect("hex");
    assert_eq!(
        content_id(&raw),
        fixture["expected_content_id"],
        "fixture digest mismatch"
    );

    let mut store = ReferenceStore::new();
    let mut executed = 0usize;
    for step in fixture["operations"].as_array().expect("operations") {
        let op_id = step["id"].as_str().expect("step id");
        let row = plan_ops
            .get(op_id)
            .unwrap_or_else(|| panic!("unmapped operation: {op_id}"));

        // Every engine capability mapping for this operation.
        let mut actual_support = serde_json::Map::new();
        for engine in ENGINES {
            let cell = row[engine].as_str().expect("plan cell");
            actual_support.insert(
                engine.to_string(),
                Value::String(support_level(op_id, engine, cell)),
            );
        }
        assert_eq!(
            Value::Object(actual_support),
            step["engine_support"],
            "engine support mismatch for {op_id}"
        );

        // The transition itself.
        let mut observed = store.execute(op_id, &step["input"], &fixture);
        if step["expected"].get("content_id").is_none() {
            // Mirrors the Python oracle: content_id is only part of the
            // observed shape when the fixture step expects it.
            observed
                .as_object_mut()
                .expect("observed object")
                .remove("content_id");
        }
        assert_eq!(
            observed, step["expected"],
            "transition mismatch for {op_id}"
        );
        executed += 1;
    }

    // Summary, mirroring benchmark-workload-oracle-output.txt.
    assert_eq!(executed, 16, "reference transitions executed");
    assert_eq!(
        executed * ENGINES.len(),
        64,
        "engine capability mappings checked"
    );
}

#[test]
fn result_schema_is_present_and_well_formed() {
    let schema = read_json(&crate_root().join("result-v1.schema.json"));
    assert_eq!(schema["title"], "id-benchmarks result v1");
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for field in [
        "schema_version",
        "engine",
        "dataset_digest",
        "environment",
        "metrics",
    ] {
        assert!(
            required.contains(&field),
            "result schema must require {field}"
        );
    }
    // The environment block carries the plan's environment_fields.
    let env_required: Vec<&str> = schema["properties"]["environment"]["required"]
        .as_array()
        .expect("environment required list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for field in [
        "runner_version",
        "git_revision",
        "hardware_model",
        "cpu_count",
        "memory_bytes",
        "os_kernel",
        "runtime_versions",
        "start_time_utc",
        "cache_mode",
        "storage_shard_count",
        "dataset_digest",
    ] {
        assert!(
            env_required.contains(&field),
            "environment must require {field}"
        );
    }
}
