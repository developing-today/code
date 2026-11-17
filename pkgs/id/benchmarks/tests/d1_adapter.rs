//! D1 adapter oracle for the #9133 benchmark contract (C9133-02-D1,
//! issue #10162): drives the synthetic workload fixture through the
//! D1 adapter's Fake D1 and checks, per operation —
//!   1. the transition equals the fixture's expected row AND the
//!      independent `ReferenceStore` transition (reference oracle:
//!      independent reference rows);
//!   2. the extracted native counters equal the expected counter
//!      fixture below (`rows_read` / `rows_written` per step, under
//!      the counting rules documented in `adapters::d1`);
//!   3. the D1 support mapping equals the fixture's `engine_support`
//!      row for `d1`.
//! No network call exists in the adapter, and no assertion touches
//! latency: the fake's synthetic duration must stay exactly 0.0.

use id_benchmarks::adapters::d1::{D1Adapter, SCHEMA};
use id_benchmarks::{support_level, ReferenceStore};
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

/// Expected counter fixture: (rows_read, rows_written) per workload
/// step, derived from the counting rules in `adapters::d1` — a PK
/// probe reads 1, a ref count reads one index entry per matching
/// ref, a scan reads every row examined, each changed row writes 1.
const EXPECTED_COUNTERS: [(u64, u64); 16] = [
    (1, 3), // 0 put create: object probe; object+generation+physical inserts
    (2, 0), // 1 put exists: object+generation probes, 0 refs counted
    (3, 0), // 2 blob_get: object+generation+physical probes
    (1, 0), // 3 metadata_lookup: object probe
    (2, 0), // 4 metadata_scan: 1 object row examined + its generation probe
    (1, 1), // 5 ref_commit_if_live: EXISTS probe; ref insert
    (3, 0), // 6 put exists: probes + 1 ref counted
    (2, 0), // 7 tombstone refused: generation probe + 1 ref counted
    (0, 1), // 8 ref_remove: ref delete; count after reads 0 remaining refs
    (1, 1), // 9 tombstone: generation probe + 0 refs; generation update
    (1, 0), // 10 ref_commit fenced: EXISTS probe, no insert
    (2, 0), // 11 put refused (tombstoned): object+generation probes
    (2, 3), // 12 resurrection: probes; generation+physical inserts, object update
    (1, 1), // 13 ref_commit_if_live gen 2: EXISTS probe; ref insert
    (3, 1), // 14 delete captured key: generation probe, physical delete, surviving physical probe + 1 ref counted
    (3, 0), // 15 blob_get gen 2: object+generation+physical probes
];

#[test]
fn d1_adapter_matches_reference_rows_and_counter_fixture() {
    let fixture = read_json(&crate_root().join("fixtures/synthetic-workload.json"));
    let plan = read_json(&design_dir().join("benchmark-plan.json"));
    let plan_ops: BTreeMap<String, Value> = plan["logical_operations"]
        .as_array()
        .expect("plan ops")
        .iter()
        .map(|row| (row["id"].as_str().expect("op id").to_string(), row.clone()))
        .collect();

    let mut reference = ReferenceStore::new();
    let mut adapter = D1Adapter::new();
    let mut total_read = 0u64;
    let mut total_written = 0u64;
    let mut size_before_delete = 0u64;

    for (step_idx, step) in fixture["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .enumerate()
    {
        let op_id = step["id"].as_str().expect("step id");
        let row = plan_ops
            .get(op_id)
            .unwrap_or_else(|| panic!("unmapped operation: {op_id}"));

        // D1 support mapping for this operation.
        let cell = row["d1"].as_str().expect("plan d1 cell");
        assert_eq!(
            support_level(op_id, "d1", cell),
            step["engine_support"]["d1"]
                .as_str()
                .expect("fixture d1 support"),
            "d1 support mismatch for step {step_idx} ({op_id})"
        );

        // The transition: adapter vs fixture vs independent reference.
        let outcome = adapter.execute(op_id, &step["input"], &fixture);
        let mut observed = outcome.transition.clone();
        if step["expected"].get("content_id").is_none() {
            observed
                .as_object_mut()
                .expect("observed object")
                .remove("content_id");
        }
        assert_eq!(
            observed, step["expected"],
            "adapter transition mismatch for step {step_idx} ({op_id})"
        );
        let mut reference_observed = reference.execute(op_id, &step["input"], &fixture);
        if step["expected"].get("content_id").is_none() {
            reference_observed
                .as_object_mut()
                .expect("reference object")
                .remove("content_id");
        }
        assert_eq!(
            observed, reference_observed,
            "adapter/reference divergence for step {step_idx} ({op_id})"
        );

        // Native counters: the expected counter fixture.
        let (want_read, want_written) = EXPECTED_COUNTERS[step_idx];
        assert_eq!(
            outcome.counters.rows_read, want_read,
            "rows_read mismatch for step {step_idx} ({op_id})"
        );
        assert_eq!(
            outcome.counters.rows_written, want_written,
            "rows_written mismatch for step {step_idx} ({op_id})"
        );
        assert_eq!(
            outcome.counters.duration_ms, 0.0,
            "fake duration must stay synthetic (no latency) at step {step_idx}"
        );
        assert!(
            !outcome.statements.is_empty(),
            "every operation must name its D1 statements (step {step_idx})"
        );
        total_read += outcome.counters.rows_read;
        total_written += outcome.counters.rows_written;
        if step_idx == 0 {
            assert!(
                outcome.counters.size_after > 0,
                "DB size must be nonzero after the first put"
            );
        }
        if step_idx == 13 {
            size_before_delete = outcome.counters.size_after;
        }
        if step_idx == 14 {
            assert!(
                outcome.counters.size_after < size_before_delete,
                "DB size must shrink after the captured-key delete"
            );
        }
    }

    assert_eq!(total_read, 28, "total rows_read over the workload");
    assert_eq!(total_written, 11, "total rows_written over the workload");
}

#[test]
fn d1_schema_covers_the_mapping_tables() {
    let schema = SCHEMA.join("\n");
    for table in [
        "objects",
        "generations",
        "generation_refs",
        "physical_objects",
    ] {
        assert!(
            schema.contains(&format!("CREATE TABLE {table}")),
            "schema must create {table}"
        );
    }
}
