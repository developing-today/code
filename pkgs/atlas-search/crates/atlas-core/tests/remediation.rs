//! #8479 no-write inventory + destination verifier (#12406) — oracle tests.
//!
//! Oracles named per the testing doctrine:
//! * `build_report` is compared against hand-computed mappings over a
//!   tiny planted set (pure function, no I/O);
//! * the transport path is checked against a local SQLite fixture whose
//!   rows are written by hand, so the expected mapping is the fixture;
//! * the resume oracle is the run id: identical inputs must reproduce it,
//!   and a changed destination must change it.

use std::path::{Path, PathBuf};

use atlas_core::filters::Sensitivity;
use atlas_core::remediation::{
    DestinationChunk, InventoryReport, MappingStatus, SourceChunk, build_report, inventory,
    read_run, write_run,
};
use atlas_core::shards::LocalShardTransport;
use atlas_core::types::DEFAULT_VEC_TABLE;

fn s(id: i64, hash: &str) -> SourceChunk {
    SourceChunk {
        id,
        content_hash: hash.to_string(),
    }
}

fn d(id: i64, hash: &str, has_vector: bool) -> DestinationChunk {
    DestinationChunk {
        id,
        content_hash: hash.to_string(),
        has_vector,
    }
}

#[test]
fn mapping_verifies_matching_content_and_reports_each_failure_kind() {
    let src = vec![s(1, "a"), s(2, "b"), s(3, "c"), s(4, "d")];
    // 1: exact match with vector -> verified.
    // 2: content present but no vector row -> missing_vector.
    // 3: same id, different hash -> hash_mismatch.
    // 4: absent entirely -> missing_destination.
    let dst = vec![
        d(1, "a", true),
        d(2, "b", false),
        d(3, "zzz", true),
        d(9, "extra", true),
    ];
    let r = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst);

    assert_eq!(r.counts.source, 4);
    assert_eq!(r.counts.destination, 4);
    assert_eq!(r.counts.verified, 1);
    assert_eq!(r.counts.missing_vector, 1);
    assert_eq!(r.counts.hash_mismatch, 1);
    assert_eq!(r.counts.missing_destination, 1);
    assert_eq!(r.unverified_source_ids, vec![2, 3, 4]);
    assert_eq!(r.destination_only_ids, vec![9]);
    assert!(!r.destination_complete());

    let statuses: Vec<MappingStatus> = r.mappings.iter().map(|m| m.status).collect();
    assert_eq!(
        statuses,
        vec![
            MappingStatus::Verified,
            MappingStatus::MissingVector,
            MappingStatus::HashMismatch,
            MappingStatus::MissingDestination,
        ]
    );
}

#[test]
fn complete_mapping_is_the_only_state_that_permits_a_visibility_switch() {
    let src = vec![s(1, "a"), s(2, "b")];
    let dst = vec![d(1, "a", true), d(2, "b", true)];
    let r = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst);
    assert!(r.destination_complete());
    assert!(r.unverified_source_ids.is_empty());

    // A destination that is a superset is still complete for the source set.
    let dst_extra = vec![d(1, "a", true), d(2, "b", true), d(3, "other", true)];
    let r2 = build_report(
        "monolith",
        "secret",
        Sensitivity::SecretBearing,
        &src,
        &dst_extra,
    );
    assert!(r2.destination_complete());
    assert_eq!(r2.destination_only_ids, vec![3]);
}

#[test]
fn run_id_is_stable_for_identical_inputs_and_changes_with_the_destination() {
    let src = vec![s(1, "a"), s(2, "b")];
    let dst = vec![d(1, "a", true), d(2, "b", true)];
    let a = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst);
    let b = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst);
    assert_eq!(a.run_id, b.run_id);

    let c = build_report(
        "monolith",
        "secret-2",
        Sensitivity::SecretBearing,
        &src,
        &dst,
    );
    assert_ne!(a.run_id, c.run_id);
}

#[test]
fn run_record_round_trips_and_refuses_a_foreign_format() {
    let r = build_report(
        "monolith",
        "secret",
        Sensitivity::SecretBearing,
        &[s(1, "a")],
        &[d(1, "a", true)],
    );
    let path = temp_path("run");
    write_run(&path, &r).expect("write");
    let back = read_run(&path).expect("read");
    assert_eq!(back, r);

    std::fs::write(&path, br#"{"format":"other/v9"}"#).expect("overwrite");
    assert!(read_run(&path).is_err(), "a foreign format must be refused");
    let _ = std::fs::remove_file(&path);
}

// ------------------------------------------------------- transport path ---

fn temp_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-remediation-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join("run.json")
}

fn create_db(path: &Path, rows: &[(i64, &str, Option<&str>, bool)]) {
    let con = rusqlite::Connection::open(path).expect("create db");
    con.execute_batch(&format!(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);"
    ))
    .expect("schema");
    for (id, hash, sens, has_vec) in rows {
        con.execute(
            "INSERT INTO chunks VALUES (?, ?, 0, ?, ?, ?)",
            rusqlite::params![id, format!("m{id}"), "REDACTED", hash, sens],
        )
        .expect("chunk");
        if *has_vec {
            con.execute(
                &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?, ?)"),
                rusqlite::params![id, vec![0u8; 4]],
            )
            .expect("vec");
        }
    }
}

#[test]
fn transport_inventory_reads_only_labelled_source_rows_and_never_text() {
    let dir = temp_path("fixture");
    let src_path = dir.with_file_name("source.db");
    let dst_path = dir.with_file_name("dest.db");
    create_db(
        &src_path,
        &[
            (1, "h1", Some("secret-bearing"), true),
            (2, "h2", Some("secret-bearing"), true),
            (3, "h3", Some("personal"), true),
        ],
    );
    create_db(
        &dst_path,
        &[
            (1, "h1", Some("secret-bearing"), true),
            // h2 is absent at the destination.
        ],
    );

    let src = LocalShardTransport::open(&src_path).expect("src");
    let dst = LocalShardTransport::open(&dst_path).expect("dst");
    let r = inventory(
        &src,
        &dst,
        "monolith",
        "atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        DEFAULT_VEC_TABLE,
    )
    .expect("inventory");

    // The personal row is not part of the secret-bearing inventory.
    assert_eq!(r.counts.source, 2);
    assert_eq!(r.counts.verified, 1);
    assert_eq!(r.counts.missing_destination, 1);
    assert_eq!(r.unverified_source_ids, vec![2]);
    assert!(!r.destination_complete());
}

#[test]
fn absence_predicate_holds_only_at_zero_remaining_rows() {
    assert!(InventoryReport::absence_holds(0));
    assert!(!InventoryReport::absence_holds(1));
}

#[test]
fn maps_by_content_hash_and_classifies_incomplete_secret_destinations() {
    // Oracle: only equal content hashes with a destination vector are verified;
    // missing hashes, id-only matches, and absent vectors stay unresolved.
    let source = [
        s(1, "hash-a"),
        s(2, "hash-b"),
        s(3, "hash-c"),
        s(4, "hash-d"),
    ];
    let destination = [
        d(11, "hash-a", true),
        d(2, "different-hash", true),
        d(12, "hash-b", false),
    ];

    let report = build_report(
        "monolith",
        "atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &destination,
    );

    assert_eq!(report.counts.source, 4);
    assert_eq!(report.counts.destination, 3);
    assert_eq!(report.counts.verified, 1);
    assert_eq!(report.counts.hash_mismatch, 1);
    assert_eq!(report.counts.missing_vector, 1);
    assert_eq!(report.counts.missing_destination, 1);
    assert_eq!(
        report
            .mappings
            .iter()
            .map(|mapping| mapping.status)
            .collect::<Vec<_>>(),
        [
            MappingStatus::Verified,
            MappingStatus::MissingVector,
            MappingStatus::HashMismatch,
            MappingStatus::MissingDestination,
        ]
    );
}
