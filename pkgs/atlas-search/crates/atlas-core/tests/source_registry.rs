//! Oracle tests for the provenance source registry (#3125 slice 4):
//! source-root and source-locator registration is stable — the same
//! registration repeated yields exactly one stored row, the first
//! registration's data wins — and registration is additive only: it
//! never alters existing data, proven against a seeded shard schema
//! (`chunks` + `chunk_occurrences`, the DDL the aggregator writes)
//! snapshotted across registrations.

use std::path::PathBuf;

use atlas_core::source_registry::{
    ensure_source_registry, locators_for_root, register_locator, register_source_root, source_root,
};
use rusqlite::{Connection, params};

const T1: &str = "2026-11-23T00:00:00Z";
const T2: &str = "2026-11-24T00:00:00Z";

fn temp_db(tag: &str) -> (PathBuf, Connection) {
    let dir = std::env::temp_dir().join(format!(
        "atlas-source-registry-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("registry.db");
    let conn = Connection::open(&path).expect("open registry db");
    (path, conn)
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .expect("count rows")
}

#[test]
fn registration_is_stable_and_idempotent() {
    let (_path, conn) = temp_db("stable");

    let first = register_source_root(
        &conn,
        "code-repo",
        "developing-today/code@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T1,
    )
    .unwrap();
    assert!(first.created);
    assert_eq!(first.root.registered_at, T1);

    // A repeat with a later clock changes nothing: the stored row is
    // the first registration's, byte for byte.
    let repeat = register_source_root(
        &conn,
        "code-repo",
        "developing-today/code@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T2,
    )
    .unwrap();
    assert!(!repeat.created);
    assert_eq!(repeat.root, first.root);
    assert_eq!(count(&conn, "source_roots"), 1);

    let loc = "developing-today/code:doc/notes.md@d85bd4ff1f3b73700f881aa99357adeae997e1bd";
    let first_loc = register_locator(&conn, "code-repo", loc, T1).unwrap();
    assert!(first_loc.created);
    let repeat_loc = register_locator(&conn, "code-repo", loc, T2).unwrap();
    assert!(!repeat_loc.created);
    assert_eq!(count(&conn, "source_locators"), 1);
    // The stored registered_at is still the first registration's.
    let stored_at: String = conn
        .query_row(
            "SELECT registered_at FROM source_locators WHERE root_id = ?1 AND locator = ?2",
            params!["code-repo", loc],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored_at, T1);
}

#[test]
fn registration_never_alters_existing_data() {
    let (_path, conn) = temp_db("non-alteration");
    // Seed the shard schema the aggregator writes (#3125 slice 2),
    // with data, in the same database the registry will live in.
    conn.execute_batch(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT); \
         CREATE TABLE chunk_occurrences ( \
           shard_chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE, \
           canonical_hash TEXT NOT NULL, source TEXT NOT NULL, \
           span_start INTEGER NOT NULL, span_end INTEGER NOT NULL, observed_at TEXT NOT NULL, \
           PRIMARY KEY (shard_chunk_id, source, span_start));",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES (1, 'msg-1', 0, 'hello provenance', 'hash-1', 'internal')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunk_occurrences VALUES (1, 'hash-1', \
         'developing-today/code:doc/notes.md@d85bd4ff1f3b73700f881aa99357adeae997e1bd', \
         0, 16, ?1)",
        params![T1],
    )
    .unwrap();

    let snapshot = |conn: &Connection| -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let dump = |table: &str, cols: &str| -> Vec<Vec<String>> {
            let mut stmt = conn
                .prepare(&format!("SELECT {cols} FROM {table} ORDER BY 1, 2"))
                .unwrap();
            let n = stmt.column_count();
            let rows = stmt
                .query_map([], |r| {
                    let mut row = Vec::new();
                    for i in 0..n {
                        let v = r.get::<_, rusqlite::types::Value>(i)?;
                        row.push(match v {
                            rusqlite::types::Value::Text(s) => s,
                            rusqlite::types::Value::Integer(v) => v.to_string(),
                            other => format!("{other:?}"),
                        });
                    }
                    Ok(row)
                })
                .unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        (
            dump(
                "chunks",
                "id, message_id, chunk_idx, text, content_hash, sensitivity",
            ),
            dump(
                "chunk_occurrences",
                "shard_chunk_id, canonical_hash, source, span_start, span_end, observed_at",
            ),
        )
    };
    let before = snapshot(&conn);

    // New registrations and repeats alike must leave the seeded
    // tables byte-identical.
    register_source_root(
        &conn,
        "code-repo",
        "developing-today/code@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T1,
    )
    .unwrap();
    register_locator(
        &conn,
        "code-repo",
        "developing-today/code:doc/notes.md@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T1,
    )
    .unwrap();
    register_source_root(
        &conn,
        "code-repo",
        "developing-today/code@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T2,
    )
    .unwrap();
    register_locator(
        &conn,
        "code-repo",
        "developing-today/code:doc/notes.md@d85bd4ff1f3b73700f881aa99357adeae997e1bd",
        T2,
    )
    .unwrap();

    assert_eq!(snapshot(&conn), before, "existing data is untouched");
}

#[test]
fn conflicting_root_repoint_is_refused() {
    let (_path, conn) = temp_db("repoint");
    register_source_root(&conn, "code-repo", "developing-today/code@aaaa", T1).unwrap();
    let err = register_source_root(&conn, "code-repo", "developing-today/code@bbbb", T2)
        .expect_err("re-pointing a root must be refused");
    assert!(
        err.to_string().contains("refusing to re-point"),
        "unexpected error: {err}"
    );
    // The original registration stands, unaltered.
    let stored = source_root(&conn, "code-repo").unwrap().unwrap();
    assert_eq!(stored.base_locator, "developing-today/code@aaaa");
    assert_eq!(stored.registered_at, T1);
    assert_eq!(count(&conn, "source_roots"), 1);
}

#[test]
fn locator_under_unknown_root_is_refused() {
    let (_path, conn) = temp_db("unknown-root");
    let err = register_locator(&conn, "nope", "some/path.md", T1)
        .expect_err("a locator under an unknown root must be refused");
    assert!(
        err.to_string().contains("unknown root"),
        "unexpected error: {err}"
    );
    assert_eq!(count(&conn, "source_locators"), 0);
}

#[test]
fn locators_resolve_under_their_root() {
    let (_path, conn) = temp_db("resolve");
    ensure_source_registry(&conn).unwrap();
    register_source_root(&conn, "code-repo", "developing-today/code@aaaa", T1).unwrap();
    register_source_root(&conn, "extracts", "laptop-extracts/sandbox-agents", T1).unwrap();
    register_locator(
        &conn,
        "code-repo",
        "developing-today/code:doc/notes.md@aaaa",
        T1,
    )
    .unwrap();
    register_locator(
        &conn,
        "code-repo",
        "developing-today/code:Cargo.toml@aaaa",
        T1,
    )
    .unwrap();
    register_locator(&conn, "extracts", "sandbox-agents/run-1/log.txt", T1).unwrap();

    // Sorted, and separated per root.
    assert_eq!(
        locators_for_root(&conn, "code-repo").unwrap(),
        vec![
            "developing-today/code:Cargo.toml@aaaa".to_string(),
            "developing-today/code:doc/notes.md@aaaa".to_string(),
        ]
    );
    assert_eq!(
        locators_for_root(&conn, "extracts").unwrap(),
        vec!["sandbox-agents/run-1/log.txt".to_string()]
    );
    // Unknown roots resolve to nothing rather than erroring.
    assert!(locators_for_root(&conn, "nope").unwrap().is_empty());
    assert!(source_root(&conn, "nope").unwrap().is_none());
    assert_eq!(
        source_root(&conn, "extracts")
            .unwrap()
            .unwrap()
            .base_locator,
        "laptop-extracts/sandbox-agents"
    );
}

#[test]
fn empty_fields_are_refused() {
    let (_path, conn) = temp_db("empty");
    assert!(register_source_root(&conn, "", "base", T1).is_err());
    assert!(register_source_root(&conn, "root", "  ", T1).is_err());
    assert!(register_source_root(&conn, "root", "base", "").is_err());
    register_source_root(&conn, "root", "base", T1).unwrap();
    assert!(register_locator(&conn, "root", "", T1).is_err());
    assert_eq!(count(&conn, "source_roots"), 1);
    assert_eq!(count(&conn, "source_locators"), 0);
}
