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

use atlas_core::error::Result;
use atlas_core::filters::Sensitivity;
use atlas_core::remediation::{
    DestinationChunk, InventoryReport, MappingStatus, SourceChunk, build_report, inventory,
    read_destination, read_run, read_source, validate_resume, write_run,
};
use atlas_core::shards::LocalShardTransport;
use atlas_core::turso::PipelineTransport;
use atlas_core::types::{DEFAULT_VEC_TABLE, Row, Stmt, Value};
use sha2::{Digest, Sha256};

fn content_hash(label: &str) -> String {
    Sha256::digest(label.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn s(id: i64, hash: &str) -> SourceChunk {
    SourceChunk {
        id,
        content_hash: content_hash(hash),
        vector_fingerprint: Some(content_hash(&format!("vector:{hash}"))),
    }
}

fn d(id: i64, hash: &str, has_vector: bool) -> DestinationChunk {
    DestinationChunk {
        id,
        content_hash: content_hash(hash),
        vector_fingerprint: has_vector.then(|| content_hash(&format!("vector:{hash}"))),
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
    let r = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst)
        .expect("valid fixture hashes");

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
    let r = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst)
        .expect("valid fixture hashes");
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
    )
    .expect("valid fixture hashes");
    assert!(r2.destination_complete());
    assert_eq!(r2.destination_only_ids, vec![3]);
}

#[test]
fn run_id_is_stable_for_identical_inputs_and_changes_with_the_destination() {
    let src = vec![s(1, "a"), s(2, "b")];
    let dst = vec![d(1, "a", true), d(2, "b", true)];
    let a = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst)
        .expect("valid fixture hashes");
    let b = build_report("monolith", "secret", Sensitivity::SecretBearing, &src, &dst)
        .expect("valid fixture hashes");
    assert_eq!(a.run_id, b.run_id);

    let c = build_report(
        "monolith",
        "secret-2",
        Sensitivity::SecretBearing,
        &src,
        &dst,
    )
    .expect("valid fixture hashes");
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
    )
    .expect("valid fixture hashes");
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

fn create_db(path: &Path, rows: &[(i64, &str, Option<&str>, Option<&str>, bool)]) {
    let con = rusqlite::Connection::open(path).expect("create db");
    con.execute_batch(&format!(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT, text TEXT);
         CREATE TABLE session_labels (session_id TEXT PRIMARY KEY, sensitivity TEXT);
         CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);"
    ))
    .expect("schema");
    for (id, hash, sens, session_sens, has_vec) in rows {
        let session_id = format!("s{id}");
        con.execute(
            "INSERT INTO chunks VALUES (?, ?, 0, ?, ?, ?)",
            rusqlite::params![id, format!("m{id}"), "REDACTED", content_hash(hash), sens],
        )
        .expect("chunk");
        con.execute(
            "INSERT INTO messages VALUES (?, ?, 'user', 'FIXTURE')",
            rusqlite::params![format!("m{id}"), session_id],
        )
        .expect("message");
        if let Some(sensitivity) = session_sens {
            con.execute(
                "INSERT INTO session_labels VALUES (?, ?)",
                rusqlite::params![session_id, sensitivity],
            )
            .expect("session label");
        }
        if *has_vec {
            con.execute(
                &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?, ?)"),
                rusqlite::params![id, vec![0u8; 768 * 4]],
            )
            .expect("vec");
        }
    }
}

fn replace_vector(path: &Path, id: i64, vector: &[u8]) {
    let con = rusqlite::Connection::open(path).expect("open fixture");
    con.execute(
        &format!("UPDATE {DEFAULT_VEC_TABLE} SET vec = ?1 WHERE chunk_id = ?2"),
        rusqlite::params![vector, id],
    )
    .expect("replace vector fixture");
}

#[test]
fn transport_inventory_reads_only_labelled_source_rows_and_never_text() {
    let dir = temp_path("fixture");
    let src_path = dir.with_file_name("source.db");
    let dst_path = dir.with_file_name("dest.db");
    create_db(
        &src_path,
        &[
            (
                1,
                "h1",
                Some("secret-bearing"),
                Some("secret-bearing"),
                true,
            ),
            (
                2,
                "h2",
                Some("secret-bearing"),
                Some("secret-bearing"),
                true,
            ),
            (3, "h3", Some("personal"), Some("personal"), true),
            (4, "h4", None, Some("secret-bearing"), true),
            (5, "h5", Some("unrecognized"), Some("personal"), true),
            (6, "h6", None, None, true),
        ],
    )
    .expect("valid fixture hashes");
    create_db(
        &dst_path,
        &[
            (
                1,
                "h1",
                Some("secret-bearing"),
                Some("secret-bearing"),
                true,
            ),
            (4, "h4", None, Some("secret-bearing"), true),
        ],
    )
    .expect("valid fixture hashes");

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
    assert_eq!(r.counts.source, 5);
    assert_eq!(r.counts.verified, 2);
    assert_eq!(r.counts.missing_destination, 3);
    assert_eq!(r.unverified_source_ids, vec![2, 5, 6]);
    assert!(!r.destination_complete());
}

#[test]
fn destination_vector_must_match_source_vector_bytes() {
    let dir = temp_path("vector-mismatch");
    let source_path = dir.with_file_name("vector-source.db");
    let destination_path = dir.with_file_name("vector-destination.db");
    let row = [(
        1,
        "same-content",
        Some("secret-bearing"),
        Some("secret-bearing"),
        true,
    )];
    create_db(&source_path, &row);
    create_db(&destination_path, &row);
    replace_vector(&source_path, 1, &vec![1u8; 768 * 4]);
    replace_vector(&destination_path, 1, &vec![2u8; 768 * 4]);

    let source = LocalShardTransport::open(&source_path).expect("source");
    let destination = LocalShardTransport::open(&destination_path).expect("destination");
    let report = inventory(
        &source,
        &destination,
        "fixture://monolith",
        "fixture://secret",
        Sensitivity::SecretBearing,
        DEFAULT_VEC_TABLE,
    )
    .expect("inventory");

    assert!(
        !report.destination_complete(),
        "vector presence alone does not verify the destination embedding"
    );
    assert_eq!(report.counts.vector_mismatch, 1);
    assert_eq!(report.mappings[0].status, MappingStatus::VectorMismatch);
    let _ = std::fs::remove_file(source_path);
    let _ = std::fs::remove_file(destination_path);
}

#[test]
fn absence_predicate_holds_only_at_zero_remaining_rows() {
    assert!(InventoryReport::absence_holds(0));
    assert!(!InventoryReport::absence_holds(1));
}

#[derive(Clone)]
struct StaticTransport {
    results: Vec<Vec<Row>>,
}

impl PipelineTransport for StaticTransport {
    fn pipeline(&self, _stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
        Ok(self.results.clone())
    }
}

struct UnexpectedQuery;

impl PipelineTransport for UnexpectedQuery {
    fn pipeline(&self, _stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
        panic!("invalid SQL identifier must be rejected before a query")
    }
}

#[test]
fn missing_or_malformed_statement_results_fail_closed() {
    let no_results = StaticTransport { results: vec![] };
    let missing_rows = StaticTransport {
        results: vec![vec![]],
    };
    let malformed_empty_sentinel = StaticTransport {
        results: vec![vec![Row::new(vec![("total".into(), Value::Integer(0))])]],
    };

    for transport in [&no_results, &missing_rows, &malformed_empty_sentinel] {
        assert!(read_source(transport, Sensitivity::SecretBearing).is_err());
        assert!(read_destination(transport, DEFAULT_VEC_TABLE).is_err());
    }

    let valid_empty = StaticTransport {
        results: vec![vec![Row::new(vec![
            ("total".into(), Value::Integer(0)),
            ("id".into(), Value::Null),
            ("h".into(), Value::Null),
            ("vec".into(), Value::Null),
        ])]],
    };
    assert!(
        read_source(&valid_empty, Sensitivity::SecretBearing)
            .expect("counted empty source")
            .is_empty()
    );
    assert!(
        read_destination(&valid_empty, DEFAULT_VEC_TABLE)
            .expect("counted empty destination")
            .is_empty()
    );
}

#[test]
fn invalid_content_hashes_are_rejected_without_echoing_the_value() {
    let invalid_hash = "not-a-valid-content-hash";
    let source_row = Row::new(vec![
        ("total".into(), Value::Integer(1)),
        ("id".into(), Value::Integer(7)),
        ("h".into(), Value::Text(invalid_hash.to_string())),
        ("vec".into(), Value::Null),
    ]);
    let source = StaticTransport {
        results: vec![vec![source_row]],
    };
    let source_error = read_source(&source, Sensitivity::SecretBearing)
        .expect_err("invalid source hash must fail closed")
        .to_string();
    assert!(!source_error.contains(invalid_hash));

    let destination_row = Row::new(vec![
        ("total".into(), Value::Integer(1)),
        ("id".into(), Value::Integer(7)),
        ("h".into(), Value::Text(invalid_hash.to_string())),
        ("vec".into(), Value::Blob(vec![0u8; 768 * 4])),
    ]);
    let destination = StaticTransport {
        results: vec![vec![destination_row]],
    };
    let destination_error = read_destination(&destination, DEFAULT_VEC_TABLE)
        .expect_err("invalid destination hash must fail closed")
        .to_string();
    assert!(!destination_error.contains(invalid_hash));

    let direct_error = build_report(
        "monolith",
        "secret",
        Sensitivity::SecretBearing,
        &[SourceChunk {
            id: 7,
            content_hash: invalid_hash.to_string(),
            vector_fingerprint: None,
        }],
        &[d(7, "valid-destination-hash", true)],
    )
    .expect_err("pure report builder must reject an untrusted hash")
    .to_string();
    assert!(!direct_error.contains(invalid_hash));
}

#[test]
fn vector_table_identifier_is_rejected_before_query() {
    let err = read_destination(
        &UnexpectedQuery,
        "chunks v ON v.id = c.id UNION SELECT id,text,1 FROM chunks --",
    )
    .expect_err("SQL input must be rejected before interpolation");
    assert!(err.to_string().contains("invalid vector table identifier"));
}

#[test]
fn resume_keeps_endpoint_and_source_identity_stable_but_rejects_destination_drift() {
    let source = [s(1, "source-hash")];
    let old = build_report(
        "turso://monolith/atlas",
        "turso://secret/atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &[d(9, "source-hash", false)],
    )
    .expect("valid fixture hashes");
    let repaired = build_report(
        "turso://monolith/atlas",
        "turso://secret/atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &[d(9, "source-hash", true)],
    )
    .expect("valid fixture hashes");
    assert_eq!(old.run_id, repaired.run_id);
    assert_ne!(
        old.destination_fingerprint,
        repaired.destination_fingerprint
    );
    assert!(validate_resume(&old, &repaired).is_err());

    let other_endpoint = build_report(
        "turso://other-monolith/atlas",
        "turso://secret/atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &[d(9, "source-hash", false)],
    )
    .expect("valid fixture hashes");
    assert_ne!(old.run_id, other_endpoint.run_id);
    assert!(validate_resume(&old, &other_endpoint).is_err());
}

#[test]
fn write_run_refuses_to_replace_an_existing_file() {
    let path = temp_path("existing");
    let sentinel = b"local replica must not be truncated";
    std::fs::write(&path, sentinel).expect("create existing output");
    let report = build_report(
        "turso://monolith/atlas",
        "turso://secret/atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &[s(1, "hash")],
        &[d(1, "hash", true)],
    )
    .expect("valid fixture hashes");

    assert!(write_run(&path, &report).is_err());
    assert_eq!(std::fs::read(&path).expect("read unchanged file"), sentinel);
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn write_run_creates_private_report_files() {
    use std::os::unix::fs::PermissionsExt;

    let path = temp_path("private");
    let report = build_report(
        "turso://monolith/atlas",
        "turso://secret/atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &[s(1, "hash")],
        &[d(1, "hash", true)],
    )
    .expect("valid fixture hashes");

    write_run(&path, &report).expect("write private report");
    let mode = std::fs::metadata(&path)
        .expect("report metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let _ = std::fs::remove_file(path);
}

#[test]
fn maps_by_content_hash_and_classifies_incomplete_secret_destinations() {
    // Oracle: only equal content and vector fingerprints are verified;
    // missing hashes, id-only matches, and absent vectors stay unresolved.
    let source = [
        s(1, "hash-a"),
        s(2, "hash-b"),
        s(3, "hash-c"),
        s(4, "hash-d"),
    ];
    let destination = [
        d(11, "hash-a", true),
        d(3, "different-hash", true),
        d(12, "hash-b", false),
    ];

    let report = build_report(
        "monolith",
        "atlas-core-secret-main-01",
        Sensitivity::SecretBearing,
        &source,
        &destination,
    )
    .expect("valid fixture hashes");

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
