//! Staging-tier tests (developing-today/code#5669, slice 1): the
//! versioned v1 schema, the provisioning tool, and the unit registry.
//! No network, no secrets, no live Turso: a staging DB is a local
//! libSQL/SQLite file (the same DDL the hosted tool applies).
//!
//! Oracles (the testing doctrine asks each test to name one):
//! * **the DDL artifact itself** — a freshly provisioned DB must
//!   report `_schema_version = 1` and exactly the v1 table set named
//!   by [`atlas_core::stage::STAGE_V1_TABLES`], and the DDL text must
//!   create every one of those five tables (so the constant and the
//!   artifact cannot drift apart);
//! * **write-then-read identity** — a provisioned unit's registry
//!   record round-trips: provision → lookup returns the same record,
//!   field for field;
//! * **state-transition as the aggregator's poll sees it** — a unit
//!   moves open → closing and the `closing` poll returns exactly it.

use std::path::PathBuf;

use atlas_core::shards::Tier;
use atlas_core::stage::{
    Registry, STAGE_V1_TABLES, UnitState, check_stage_schema, provision_staging_db, schema_version,
    user_tables,
};

/// A per-test scratch directory (the shards tests use the same
/// process-id-tagged shape rather than adding a tempfile dependency).
fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-stage-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

const NOW: &str = "2026-11-08T00:00:00Z";

/// Oracle: the DDL artifact. A provisioned DB reports
/// `_schema_version = 1` and exactly the v1 table set.
#[test]
fn provisioned_db_reports_v1_schema_version_and_table_set() {
    let dir = temp_dir("schema-invariant");
    let db = dir.join("unit.db");

    provision_staging_db(&db).expect("provision");

    let conn = rusqlite::Connection::open(&db).expect("open provisioned");
    assert_eq!(schema_version(&conn).expect("version"), 1);
    assert!(
        check_stage_schema(&conn).is_ok(),
        "a freshly provisioned DB must satisfy the v1 schema invariant"
    );

    let want: std::collections::BTreeSet<&str> = STAGE_V1_TABLES.into_iter().collect();
    let got = user_tables(&conn).expect("tables");
    let got: std::collections::BTreeSet<&str> = got.iter().map(String::as_str).collect();
    assert_eq!(got, want, "table set must be exactly the v1 set");
}

/// Oracle: the DDL artifact. The compiled-in artifact text really
/// creates every table the constant names — the constant and the file
/// cannot drift.
#[test]
fn v1_ddl_artifact_creates_every_named_table() {
    let ddl = atlas_core::stage::STAGE_V1_DDL;
    for table in STAGE_V1_TABLES {
        assert!(
            ddl.contains(&format!("CREATE TABLE {table}")),
            "atlas_stage_v1.sql must create table `{table}`"
        );
    }
    // And it records the same version the constant expects.
    assert!(
        ddl.contains("INSERT INTO _schema_version (version) VALUES (1)"),
        "atlas_stage_v1.sql must record version 1"
    );
}

/// Oracle: the DDL artifact is not silently re-applicable — a
/// re-provision over a populated DB is refused rather than
/// half-clobbering a unit.
#[test]
fn reprovision_over_existing_db_is_refused() {
    let dir = temp_dir("reprovision");
    let db = dir.join("unit.db");
    provision_staging_db(&db).expect("first provision");
    let err = provision_staging_db(&db).expect_err("second provision must fail");
    assert!(
        err.to_string().contains("refusing to re-provision"),
        "unexpected error: {err}"
    );
}

/// Oracle: write-then-read identity. provision → lookup returns the
/// same record, field for field.
#[test]
fn registry_round_trips_a_provisioned_unit() {
    let dir = temp_dir("roundtrip");
    let reg = Registry::in_memory().expect("registry");

    let record = reg
        .provision(
            &dir.join("unit-a.db"),
            "task-12742",
            "repo-worker-code",
            Tier::Internal,
            NOW,
        )
        .expect("provision + register");

    assert_eq!(record.unit_id, "task-12742");
    assert_eq!(record.state, UnitState::Open);
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.sensitivity, Tier::Internal);

    let got = reg
        .lookup("task-12742")
        .expect("lookup")
        .expect("unit present");
    assert_eq!(got, record, "registry record must round-trip unchanged");

    assert!(
        reg.lookup("missing").expect("lookup").is_none(),
        "an unregistered unit id must not resolve"
    );
}

/// Oracle: the aggregator's poll. A unit moved to `closing` is exactly
/// what `units_in_state(closing)` returns; `open` no longer lists it.
#[test]
fn state_transition_moves_a_unit_between_polls() {
    let dir = temp_dir("state");
    let reg = Registry::in_memory().expect("registry");
    reg.provision(
        &dir.join("u.db"),
        "epic-5669",
        "epic-owner",
        Tier::Internal,
        NOW,
    )
    .expect("provision");

    assert_eq!(reg.units_in_state(UnitState::Open).expect("open").len(), 1);
    assert!(
        reg.units_in_state(UnitState::Closing)
            .expect("closing")
            .is_empty()
    );

    reg.set_state("epic-5669", UnitState::Closing, "2026-11-08T00:05:00Z")
        .expect("close");

    assert!(
        reg.units_in_state(UnitState::Open)
            .expect("open")
            .is_empty()
    );
    let closing = reg.units_in_state(UnitState::Closing).expect("closing");
    assert_eq!(closing.len(), 1);
    assert_eq!(closing[0].unit_id, "epic-5669");
    assert_eq!(closing[0].state, UnitState::Closing);

    assert!(
        reg.set_state("missing", UnitState::Merged, NOW).is_err(),
        "moving an unknown unit must fail loudly"
    );
}

/// Oracle: the registry as read from another process. Provisioning
/// with a relative `--db` path must persist an absolute db_uri —
/// a relative one would resolve against whatever working directory
/// later reads the registry (Codex P2, PR #7365).
#[test]
fn provision_persists_an_absolute_db_path() {
    let rel = PathBuf::from(format!("atlas-stage-relative-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&rel);
    let reg = Registry::in_memory().expect("registry");

    let record = reg
        .provision(
            &rel.join("unit.db"),
            "task-12742-rel",
            "repo-worker-code",
            Tier::Internal,
            NOW,
        )
        .expect("provision with a relative path");

    let db_path = PathBuf::from(&record.db_uri);
    assert!(
        db_path.is_absolute(),
        "db_uri must be persisted absolute, got {}",
        record.db_uri
    );
    assert!(
        db_path.exists(),
        "the persisted db_uri must name the provisioned DB"
    );
    let got = reg
        .lookup("task-12742-rel")
        .expect("lookup")
        .expect("unit present");
    assert_eq!(got.db_uri, record.db_uri);
    let _ = std::fs::remove_dir_all(&rel);
}
