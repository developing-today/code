//! Classification manifest tests (developing-today/code#6993, phase 2
//! of the #6228 manifest programme).
//!
//! Oracles: the design record
//! `doc/2026-11-05T13-03-49Z_design_atlas_manifest_filter_surface` —
//! §3 (manifest home + stored shape), §4 (shape vocabulary +
//! generic_list store), §5 (exact semantics as decoded element
//! equality), §6 (what a manifest can never do), §7 (fall back to
//! the built-in manifest, loudly, never widen).

use atlas_core::manifest::{
    Dimension, GENERIC_LABELS_DDL, Manifest, ManifestSource, Semantics, Shape, builtin_manifest,
    generic_list_filter, load_manifest,
};
use rusqlite::Connection;

fn dim(name: &str, shape: Shape, semantics: Semantics, col: Option<&str>) -> Dimension {
    Dimension {
        name: name.to_string(),
        shape,
        semantics,
        label_column: col.map(str::to_string),
    }
}

fn manifest_with(dimensions: Vec<Dimension>) -> Manifest {
    let mut m = Manifest {
        version: 1,
        content_hash: String::new(),
        dimensions,
    };
    m.content_hash = m.compute_hash();
    m
}

/// Write a manifest into `con` in the stored shape (§3): a header row
/// (name NULL) plus one row per dimension, all repeating version+hash.
fn write_manifest_table(con: &Connection, m: &Manifest) {
    con.execute(
        "CREATE TABLE classification_manifest (manifest_version INTEGER, content_hash TEXT, \
         name TEXT, shape TEXT, semantics TEXT, label_column TEXT)",
        [],
    )
    .expect("create manifest table");
    con.execute(
        "INSERT INTO classification_manifest VALUES (?, ?, NULL, NULL, NULL, NULL)",
        rusqlite::params![m.version, m.content_hash],
    )
    .expect("header row");
    for d in &m.dimensions {
        con.execute(
            "INSERT INTO classification_manifest VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                m.version,
                m.content_hash,
                d.name,
                d.shape.name(),
                d.semantics.name(),
                d.label_column
            ],
        )
        .expect("dimension row");
    }
}

#[test]
fn builtin_manifest_expresses_todays_surface() {
    let m = builtin_manifest();
    assert_eq!(m.version, 1);
    let got: Vec<(&str, Shape, Option<&str>)> = m
        .dimensions
        .iter()
        .map(|d| (d.name.as_str(), d.shape, d.label_column.as_deref()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("kind", Shape::ScalarIn, Some("kind")),
            ("client", Shape::ScalarIn, Some("client")),
            ("topics", Shape::JsonList, Some("topics")),
            ("projects", Shape::JsonList, Some("projects")),
            ("created_at", Shape::TimestampRange, Some("created_at")),
        ]
    );
    assert!(m.dimensions.iter().all(|d| d.semantics == Semantics::Exact));
    m.validate().expect("built-in manifest validates");
    assert_eq!(m.surface_identity(false), "builtin");
    assert!(m.surface_identity(true).starts_with("manifest v1:"));
    assert_eq!(
        m.compute_hash(),
        m.content_hash,
        "hash is stable and self-consistent"
    );
}

#[test]
fn validation_rejects_what_a_manifest_can_never_do() {
    // §6.1: no sensitivity keys exist in the schema — naming one is an error.
    let m = manifest_with(vec![dim(
        "sensitivity",
        Shape::ScalarIn,
        Semantics::Exact,
        Some("kind"),
    )]);
    assert!(m.validate().is_err());
    let m = manifest_with(vec![dim(
        "sens_tier",
        Shape::ScalarIn,
        Semantics::Exact,
        Some("kind"),
    )]);
    assert!(m.validate().is_err());
    // §6.2: column shapes bind only to the code-known allowlist.
    let m = manifest_with(vec![dim(
        "secret_col",
        Shape::ScalarIn,
        Semantics::Exact,
        Some("password"),
    )]);
    assert!(m.validate().is_err());
    // §6.2: a generic_list key may not collide with a built-in dimension.
    let m = manifest_with(vec![dim(
        "topics",
        Shape::GenericList,
        Semantics::Exact,
        None,
    )]);
    assert!(m.validate().is_err());
    // §6.2: generic keys follow the code-side pattern.
    let m = manifest_with(vec![dim(
        "Source App!",
        Shape::GenericList,
        Semantics::Exact,
        None,
    )]);
    assert!(m.validate().is_err());
    // generic_list binds the store, never a column.
    let m = manifest_with(vec![dim(
        "source_app",
        Shape::GenericList,
        Semantics::Exact,
        Some("kind"),
    )]);
    assert!(m.validate().is_err());
    // duplicates are rejected.
    let m = manifest_with(vec![
        dim("kind", Shape::ScalarIn, Semantics::Exact, Some("kind")),
        dim("kind", Shape::ScalarIn, Semantics::Exact, Some("kind")),
    ]);
    assert!(m.validate().is_err());
    // a well-formed generic_list dimension is accepted.
    let m = manifest_with(vec![dim(
        "source_app",
        Shape::GenericList,
        Semantics::Exact,
        None,
    )]);
    m.validate().expect("generic_list dimension validates");
}

#[test]
fn loader_absent_table_is_builtin_without_warning() {
    let con = Connection::open_in_memory().expect("db");
    let loaded = load_manifest(&con, &["main"]);
    assert_eq!(loaded.source, ManifestSource::BuiltinAbsent);
    assert!(loaded.warning.is_none());
    assert_eq!(loaded.manifest, builtin_manifest());
}

#[test]
fn loader_reads_a_valid_manifest_including_generic_dimensions() {
    let con = Connection::open_in_memory().expect("db");
    let mut dims = builtin_manifest().dimensions;
    dims.push(dim(
        "source_app",
        Shape::GenericList,
        Semantics::Exact,
        None,
    ));
    let stored = manifest_with(dims);
    write_manifest_table(&con, &stored);
    let loaded = load_manifest(&con, &["main"]);
    assert_eq!(
        loaded.source,
        ManifestSource::Table {
            schema: "main".to_string()
        }
    );
    assert!(loaded.warning.is_none());
    assert_eq!(loaded.manifest, stored);
    assert!(loaded.describe().contains("classification_manifest (main)"));
}

#[test]
fn loader_falls_back_loudly_on_hash_mismatch_and_newer_version() {
    // §7: malformed (hash mismatch) → built-in + warning.
    let con = Connection::open_in_memory().expect("db");
    let mut stored = manifest_with(builtin_manifest().dimensions);
    stored.content_hash = "deadbeef".to_string();
    write_manifest_table(&con, &stored);
    let loaded = load_manifest(&con, &["main"]);
    assert_eq!(loaded.source, ManifestSource::BuiltinFallback);
    assert!(loaded.warning.expect("warning").contains("content_hash"));
    assert_eq!(loaded.manifest, builtin_manifest());

    // §7: newer than the binary understands → built-in + warning.
    let con = Connection::open_in_memory().expect("db");
    let mut stored = manifest_with(builtin_manifest().dimensions);
    stored.version = 99;
    stored.content_hash = stored.compute_hash();
    write_manifest_table(&con, &stored);
    let loaded = load_manifest(&con, &["main"]);
    assert_eq!(loaded.source, ManifestSource::BuiltinFallback);
    assert!(loaded.warning.expect("warning").contains("newer"));
}

#[test]
fn loader_falls_back_on_unknown_shape_and_sensitivity_rows() {
    let con = Connection::open_in_memory().expect("db");
    con.execute(
        "CREATE TABLE classification_manifest (manifest_version INTEGER, content_hash TEXT, \
         name TEXT, shape TEXT, semantics TEXT, label_column TEXT)",
        [],
    )
    .expect("table");
    con.execute(
        "INSERT INTO classification_manifest VALUES (1, 'x', NULL, NULL, NULL, NULL), \
         (1, 'x', 'kind', 'raw_sql', 'exact', 'kind')",
        [],
    )
    .expect("rows");
    let loaded = load_manifest(&con, &["main"]);
    assert_eq!(loaded.source, ManifestSource::BuiltinFallback);
    assert!(loaded.warning.is_some());
}

#[test]
fn generic_list_exact_matching_is_decoded_element_equality() {
    let con = Connection::open_in_memory().expect("db");
    con.execute_batch(GENERIC_LABELS_DDL).expect("store ddl");
    con.execute("CREATE TABLE sessions (id TEXT PRIMARY KEY)", [])
        .expect("sessions");
    con.execute_batch(
        "INSERT INTO sessions VALUES ('s1'), ('s2');
         INSERT INTO generic_labels VALUES ('s1', 'topics', '[\"a\", \"b\"]');
         INSERT INTO generic_labels VALUES ('s2', 'topics', '[\"rust\"]');",
    )
    .expect("rows");

    let run = |values: &[String]| -> Vec<String> {
        let (frag, params) = generic_list_filter("topics", Semantics::Exact, values, "s.id");
        let sql = format!("SELECT s.id FROM sessions s WHERE {frag} ORDER BY s.id");
        let mut stmt = con.prepare(&sql).expect("prepare");
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                r.get::<_, String>(0)
            })
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    };

    assert_eq!(run(&["a".to_string()]), vec!["s1"]);
    // §5's boundary case: the value a", "b must never line up with the
    // two elements "a", "b".
    assert!(run(&["a\", \"b".to_string()]).is_empty());
    assert_eq!(
        run(&["rust".to_string(), "a".to_string()]),
        vec!["s1", "s2"]
    );
}

#[test]
fn generic_list_pattern_matching_translates_globs() {
    let con = Connection::open_in_memory().expect("db");
    con.execute_batch(GENERIC_LABELS_DDL).expect("store ddl");
    con.execute("CREATE TABLE sessions (id TEXT PRIMARY KEY)", [])
        .expect("sessions");
    con.execute_batch(
        "INSERT INTO sessions VALUES ('s1'), ('s2'), ('s3');
         INSERT INTO generic_labels VALUES ('s1', 'topics', '[\"rust\"]');
         INSERT INTO generic_labels VALUES ('s2', 'topics', '[\"a%b\"]');
         INSERT INTO generic_labels VALUES ('s3', 'topics', '[\"axb\"]');",
    )
    .expect("rows");
    let run = |values: &[String]| -> usize {
        let (frag, params) = generic_list_filter("topics", Semantics::Pattern, values, "s.id");
        let sql = format!("SELECT COUNT(*) FROM sessions s WHERE {frag}");
        con.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, i64>(0)
        })
        .expect("query") as usize
    };
    assert_eq!(run(&["ru*".to_string()]), 1);
    assert_eq!(run(&["go*".to_string()]), 0);
    // §5: LIKE metacharacters in the glob's literal text stay literal —
    // "a%b" names the one element with a literal %, never "axb".
    assert_eq!(run(&["a%b".to_string()]), 1);
    assert_eq!(
        run(&["a_b".to_string()]),
        0,
        "an underscore in the glob is a literal underscore"
    );
    // …while the glob's own wildcards work: ? is exactly one character.
    assert_eq!(run(&["a?b".to_string()]), 2);
}
