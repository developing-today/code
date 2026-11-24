//! Transaction fixture for the Turso workload adapter
//! (slice C9133-02-turso, issue #10165).
//!
//! Oracle: **reference** — the Turso adapter must return the same
//! logical rows and generation transitions as the benchmark
//! reference model (C9133-01), and must expose sync
//! revision/push/pull/network state separately from the
//! transitions. This test executes the synthetic workload fixture
//! against both a real local Turso database and the reference
//! store, step by step, and requires:
//!   1. every observed transition equals the fixture's expected
//!      transition and the reference store's transition;
//!   2. the plan's Turso cell maps to the fixture's recorded
//!      Turso support level for the operation;
//!   3. the sync state captured after each step equals the
//!      expected sync fixture below, under the rules documented
//!      in `adapters::turso`: the local revision counts committed
//!      write transactions (reads and refused / no-change
//!      operations never advance it), a local-only store pushes
//!      and pulls nothing, so every committed revision is pending
//!      push and the network posture is `local-only`.
//! The test uses no remote credentials and asserts no timing.
//!
//! Run: `cargo test --manifest-path pkgs/id/benchmarks/Cargo.toml --test turso_adapter`

use id_benchmarks::adapters::turso::{NETWORK_LOCAL_ONLY, TursoStore};
use id_benchmarks::{ReferenceStore, content_id, support_level};
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

/// Expected local revision after each workload step, derived from
/// the sync rules in `adapters::turso` — a step advances the local
/// revision exactly when its transaction changes data.
const EXPECTED_LOCAL_REVISIONS: [u64; 16] = [
    1, // 0 put create: object+generation+physical inserts commit
    1, // 1 put exists: idempotent, nothing changes
    1, // 2 blob_get: read; notes the sync revision, never advances it
    1, // 3 metadata_lookup: read
    1, // 4 metadata_scan: read
    2, // 5 ref_commit_if_live: reference insert commits
    2, // 6 put exists: idempotent, nothing changes
    2, // 7 tombstone refused (references present): nothing changes
    3, // 8 ref_remove: reference delete commits
    4, // 9 tombstone: generation update commits
    4, // 10 ref_commit fenced: refused, nothing changes
    4, // 11 put refused (tombstoned): nothing changes
    5, // 12 resurrection: generation+physical inserts, object update commit
    6, // 13 ref_commit_if_live gen 2: reference insert commits
    7, // 14 delete captured key: physical delete commits
    7, // 15 blob_get gen 2: read
];

#[test]
fn turso_adapter_matches_reference_oracle_and_sync_fixture() {
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
    let mut turso = TursoStore::temp();
    assert!(turso.path().exists(), "temp turso database file exists");
    // A fresh local-only store starts at revision zero with
    // nothing pushed or pulled.
    let initial = turso.sync_state();
    assert_eq!(initial.local_revision, 0);
    assert_eq!(initial.pushed_revision, 0);
    assert_eq!(initial.pulled_revision, 0);
    assert_eq!(initial.pending_push_revisions, 0);
    assert_eq!(initial.network, NETWORK_LOCAL_ONLY);

    let mut executed = 0usize;
    for (idx, step) in fixture["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .enumerate()
    {
        let op_id = step["id"].as_str().expect("step id");
        let row = plan_ops
            .get(op_id)
            .unwrap_or_else(|| panic!("unmapped operation: {op_id}"));

        // The plan's turso cell must map to the fixture's recorded
        // turso support level for this operation.
        let cell = row["turso"].as_str().expect("turso plan cell");
        assert_eq!(
            Value::String(support_level(op_id, "turso", cell)),
            step["engine_support"]["turso"],
            "turso support mismatch for {op_id}"
        );

        let expected = &step["expected"];
        let reference_observed =
            normalized(reference.execute(op_id, &step["input"], &fixture), expected);
        let outcome = turso.execute(op_id, &step["input"], &fixture);
        let turso_observed = normalized(outcome.transition, expected);
        assert_eq!(
            turso_observed, *expected,
            "fixture transition mismatch for {op_id}"
        );
        assert_eq!(
            turso_observed, reference_observed,
            "turso adapter diverged from the reference model for {op_id}"
        );

        // Sync state is reported separately from the transition:
        // the local revision follows the sync fixture, and a
        // local-only store has pushed and pulled nothing, so all
        // committed revisions are pending push.
        let sync = outcome.sync;
        assert_eq!(
            sync.local_revision, EXPECTED_LOCAL_REVISIONS[idx],
            "local revision after step {idx} ({op_id})"
        );
        assert_eq!(sync.pushed_revision, 0, "pushed revision at step {idx}");
        assert_eq!(sync.pulled_revision, 0, "pulled revision at step {idx}");
        assert_eq!(
            sync.pending_push_revisions, sync.local_revision,
            "pending push revisions at step {idx}"
        );
        assert_eq!(sync.network, NETWORK_LOCAL_ONLY, "network at step {idx}");
        // The snapshot after the operation is the store's state.
        assert_eq!(turso.sync_state(), sync, "sync state at step {idx}");
        executed += 1;
    }
    assert_eq!(executed, 16, "turso transitions executed");
}

#[test]
fn turso_adapter_persists_across_reopen() {
    let fixture_bytes = std::fs::read(crate_root().join("fixtures/synthetic-workload.json"))
        .expect("crate fixture");
    let fixture: Value = serde_json::from_slice(&fixture_bytes).expect("parse fixture");
    let ops = fixture["operations"].as_array().expect("operations");

    let path = std::env::temp_dir().join(format!(
        "id-benchmarks-turso-reopen-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    {
        let mut store = TursoStore::open(&path);
        // Steps 0 (put), 5 (ref_commit_if_live) from the fixture.
        for idx in [0usize, 5] {
            let step = &ops[idx];
            let outcome = store.execute(
                step["id"].as_str().expect("step id"),
                &step["input"],
                &fixture,
            );
            let observed = normalized(outcome.transition, &step["expected"]);
            assert_eq!(observed, step["expected"], "step {idx} before reopen");
        }
        assert_eq!(store.sync_state().local_revision, 2);
    }
    {
        // A fresh open of the same database file must observe the
        // committed transactions AND the captured sync state:
        // generation 1, Live, one reference, local revision 2
        // with both revisions still pending push (local-only).
        let mut store = TursoStore::open(&path);
        let sync = store.sync_state();
        assert_eq!(sync.local_revision, 2, "local revision persisted");
        assert_eq!(sync.pushed_revision, 0);
        assert_eq!(sync.pulled_revision, 0);
        assert_eq!(sync.pending_push_revisions, 2);
        assert_eq!(sync.network, NETWORK_LOCAL_ONLY);

        let get = store.execute("blob_get", &serde_json::json!({}), &fixture);
        assert_eq!(get.transition, ops[2]["expected"], "blob_get after reopen");
        assert_eq!(get.sync.local_revision, 2, "read notes the sync revision");
        let lookup = store.execute("metadata_lookup", &serde_json::json!({}), &fixture);
        assert_eq!(
            lookup.transition, ops[3]["expected"],
            "metadata_lookup after reopen"
        );
        // Idempotent put reports the persisted reference count and
        // does not advance the local revision.
        let put_outcome = store.execute("blob_put_if_absent", &ops[0]["input"], &fixture);
        let put = normalized(put_outcome.transition, &ops[6]["expected"]);
        assert_eq!(
            put, ops[6]["expected"],
            "reference count persisted after reopen"
        );
        assert_eq!(put_outcome.sync.local_revision, 2);
    }
    let _ = std::fs::remove_file(&path);
}
