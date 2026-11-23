//! Metadata version conformance fixtures for Atlas (issue #9095,
//! design `doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1`).
//!
//! Oracle: the hand-authored fixture set at
//! `doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/fixtures/atlas-metadata-v1.json`
//! — every historical shape of Atlas item metadata, from the
//! unversioned `atlas_stage` v1 producer meta bag through the doctrine
//! v1 envelope. For each executable case this test inserts the case's
//! stage row into a freshly provisioned `atlas_stage` v1 database
//! (the staging design's write path is unmediated: the unit's owner
//! writes rows directly) and reads it back through the current reader,
//! `list_staged_items`. The stored `meta` must round-trip byte-exact —
//! old metadata remains valid and unrewritten under a newer system —
//! and the database schema version must be 1.
//!
//! Cases marked `executable: false` in the fixture carry a
//! `doctrine_projection` expectation (extras relocation, lossless-only
//! coercion, fail-closed sensitivity fallback). No current reader
//! projects; those cases are the recorded oracle for the design's §7
//! slices 2 and 6 and are deliberately not asserted here.

use std::path::PathBuf;

use atlas_core::stage::{list_staged_items_in_db, provision_staging_db, schema_version};
use rusqlite::Connection;
use serde_json::Value;

fn fixture_set() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../doc/2026-11-21T17-56-24Z_design_atlas_metadata_schema_v1/fixtures/atlas-metadata-v1.json"
    );
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("conformance fixture set unreadable at {path}: {e}"));
    serde_json::from_str(&raw).expect("fixture set is valid JSON")
}

fn temp_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "atlas-metadata-conformance-{}-{}-{tag}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("stage.db")
}

#[test]
fn atlas_metadata_version_conformance_fixtures() {
    let set = fixture_set();
    let cases = set["cases"].as_array().expect("fixture set carries cases");
    assert!(!cases.is_empty(), "fixture set must not be empty");
    let mut executed = 0;
    for case in cases {
        let id = case["id"].as_str().expect("case id");
        if case["executable"].as_bool() != Some(true) {
            continue;
        }
        executed += 1;
        let row = &case["stage_row"];
        let db = temp_db(id);
        provision_staging_db(&db).unwrap_or_else(|e| panic!("{id}: provision failed: {e}"));

        let conn = Connection::open(&db).unwrap();
        assert_eq!(
            schema_version(&conn).unwrap(),
            1,
            "{id}: staging schema version"
        );
        conn.execute(
            "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                row["unit_id"].as_str().expect("unit_id"),
                row["kind"].as_str().expect("kind"),
                row["source"].as_str().expect("source"),
                row["text"].as_str().expect("text"),
                row["sensitivity"].as_str().expect("sensitivity"),
                "2026-11-23T00:00:00Z",
                row["meta"].as_str().expect("meta"),
            ],
        )
        .unwrap_or_else(|e| panic!("{id}: insert failed: {e}"));
        drop(conn);

        let items = list_staged_items_in_db(&db, Some(row["unit_id"].as_str().unwrap()))
            .unwrap_or_else(|e| panic!("{id}: readback failed: {e}"));
        assert_eq!(items.len(), 1, "{id}: exactly one staged item");
        let meta = items[0].meta.as_deref().expect("meta present");
        assert_eq!(
            meta,
            row["meta"].as_str().unwrap(),
            "{id}: meta must round-trip byte-exact, unrewritten"
        );
        // And it must still parse as the fixture's object: usable, not
        // merely stored.
        let parsed: Value = serde_json::from_str(meta).unwrap();
        assert_eq!(parsed, case["object"], "{id}: meta parses to the fixture object");

        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
    assert!(executed >= 4, "expected at least 4 executable cases, ran {executed}");
}
