//! Derived-visibility fixture for the Atlas lexical branch (#13583,
//! slice 2; policy record in
//! `doc/2026-11-26T13-00-00Z_design_derived_visibility_profiles_13583/`).
//!
//! Two stores hold the same rows a reader may see. Store B also holds
//! rows above that reader's ceiling. The expected observations below
//! are written out by hand from the corpus, not computed by the code
//! under test. They pin three things about the production FTS
//! statement ([`fts_stmt`]):
//!
//! 1. **Membership and count are strict.** For every ceiling below
//!    `secret-bearing`, the full visible result set and its count are
//!    the same in both stores, and no hidden row is ever returned.
//! 2. **Lexical ranking uses shared statistics.** `bm25(chunks_fts)`
//!    takes its document frequencies from the whole index, hidden rows
//!    included. The fixture is built so that the hidden rows reverse
//!    the order of two visible rows, so the order (and therefore the
//!    rows inside a truncated pool) can differ between the stores.
//!    This is the declared looser contract of the policy record, not a
//!    strict-reader guarantee. If ranking ever becomes strict, this
//!    assertion fails and the policy record must be updated with it.
//! 3. **A partitioned index meets the strict profile.** Indexing only
//!    the rows at or below the reader's ceiling, as the vector tier
//!    shards do physically, gives identical order in both stores.
//!
//! A separate check confirms the full-trust (`secret-bearing`) view of
//! store B still returns the hidden rows. No timing claim is made.

use atlas_core::Filters;
use atlas_core::filters::{TURSO_SHAPE, fts_stmt};
use atlas_core::types::{Stmt, Value};
use rusqlite::Connection;
use rusqlite::types::Value as RValue;

/// One corpus row: chunk id, text, row sensitivity.
type Row = (i64, &'static str, &'static str);

/// Rows every store holds. All texts are three tokens long, so the
/// length normalisation is the same for every row and only the
/// document frequencies decide the BM25 order.
const SHARED: &[Row] = &[
    (1, "alpha filler filler", "shareable"),
    (2, "gamma filler filler", "shareable"),
    (3, "alpha spare words", "shareable"),
    (4, "plain spare words", "shareable"),
    (5, "plain spare words", "shareable"),
    (6, "plain spare words", "shareable"),
    (7, "plain spare words", "shareable"),
    (8, "plain spare words", "shareable"),
];

/// Rows only store B holds, all above the default `personal` ceiling.
const HIDDEN: &[Row] = &[
    (9, "gamma filler filler", "secret-bearing"),
    (10, "gamma filler filler", "secret-bearing"),
    (11, "gamma filler filler", "local-only"),
];

const QUERY: &str = "alpha OR gamma";

/// Build an in-memory store with the columns the FTS statement reads.
/// `index_ceiling`, when set, keeps rows above that ceiling out of the
/// FTS index (the partitioned profile); the rows still exist in
/// `chunks`.
fn store(rows: &[Row], index_ceiling: Option<&str>) -> Connection {
    let con = Connection::open_in_memory().expect("open in-memory store");
    con.execute_batch(
        "CREATE TABLE sessions (id TEXT PRIMARY KEY, client TEXT, created_at TEXT);
         CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL);
         CREATE TABLE session_labels (session_id TEXT PRIMARY KEY, kind TEXT, \
             topics TEXT, projects TEXT);
         CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
             text TEXT NOT NULL, sensitivity TEXT);
         CREATE VIRTUAL TABLE chunks_fts USING fts5(text);
         INSERT INTO sessions VALUES ('s1', 'cli', '2026-11-20T00:00:00Z');
         INSERT INTO messages VALUES ('m1', 's1');
         INSERT INTO session_labels VALUES ('s1', 'work', '[]', '[]');",
    )
    .expect("create schema");
    let allowed: Option<Vec<&str>> = index_ceiling.map(|c| {
        atlas_core::Sensitivity::from_name(c)
            .expect("known ceiling")
            .allowed()
    });
    for (id, text, sens) in rows {
        con.execute(
            "INSERT INTO chunks (id, message_id, text, sensitivity) VALUES (?1, 'm1', ?2, ?3)",
            rusqlite::params![id, text, sens],
        )
        .expect("insert chunk");
        let indexed = allowed.as_ref().is_none_or(|a| a.contains(sens));
        if indexed {
            con.execute(
                "INSERT INTO chunks_fts (rowid, text) VALUES (?1, ?2)",
                rusqlite::params![id, text],
            )
            .expect("index chunk");
        }
    }
    con
}

fn rvalue(v: &Value) -> RValue {
    match v {
        Value::Null => RValue::Null,
        Value::Integer(i) => RValue::Integer(*i),
        Value::Float(f) => RValue::Real(*f),
        Value::Text(s) => RValue::Text(s.clone()),
        Value::Blob(b) => RValue::Blob(b.clone()),
    }
}

fn run(con: &Connection, stmt: &Stmt) -> Vec<i64> {
    let mut prepared = con.prepare(&stmt.sql).expect("prepare FTS statement");
    let args: Vec<RValue> = stmt.args.iter().map(rvalue).collect();
    prepared
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            r.get::<_, i64>(0)
        })
        .expect("run FTS statement")
        .map(|r| r.expect("row"))
        .collect()
}

fn search(con: &Connection, ceiling: &str, pool: usize) -> Vec<i64> {
    let filters = Filters {
        max_sensitivity: atlas_core::Sensitivity::from_name(ceiling).expect("known ceiling"),
        ..Filters::default()
    };
    run(con, &fts_stmt(TURSO_SHAPE, &filters, QUERY, pool))
}

fn sorted(mut ids: Vec<i64>) -> Vec<i64> {
    ids.sort_unstable();
    ids
}

fn position(ids: &[i64], id: i64) -> usize {
    ids.iter()
        .position(|x| *x == id)
        .unwrap_or_else(|| panic!("row {id} missing from {ids:?}"))
}

fn store_b_rows() -> Vec<Row> {
    SHARED.iter().chain(HIDDEN.iter()).copied().collect()
}

#[test]
fn hidden_rows_never_change_visible_membership_or_count() {
    let a = store(SHARED, None);
    let b = store(&store_b_rows(), None);
    // Hand-derived: rows 1 and 3 hold "alpha", row 2 holds "gamma";
    // rows 4 to 8 match neither term.
    let expected = vec![1, 2, 3];
    for ceiling in ["shareable", "personal", "local-only"] {
        let in_a = sorted(search(&a, ceiling, 50));
        let mut in_b = sorted(search(&b, ceiling, 50));
        if ceiling == "local-only" {
            // Row 11 is local-only: authorized at this ceiling, so it
            // is part of the visible set, not a hidden row.
            assert_eq!(in_b, vec![1, 2, 3, 11], "local-only reader of store B");
            in_b.retain(|id| *id != 11);
        }
        assert_eq!(in_a, expected, "{ceiling} reader of store A");
        assert_eq!(in_b, expected, "{ceiling} reader of store B");
        let hidden_above: Vec<i64> = HIDDEN
            .iter()
            .filter(|(_, _, s)| {
                !atlas_core::Sensitivity::from_name(ceiling)
                    .unwrap()
                    .allowed()
                    .contains(s)
            })
            .map(|(id, _, _)| *id)
            .collect();
        for id in hidden_above {
            assert!(
                !search(&b, ceiling, 50).contains(&id),
                "hidden row {id} leaked to a {ceiling} reader"
            );
        }
    }
}

#[test]
fn shared_lexical_statistics_can_reorder_visible_rows() {
    let a = store(SHARED, None);
    let b = store(&store_b_rows(), None);
    // Store A: "gamma" is in 1 of 8 rows and "alpha" in 2 of 8, so
    // gamma is the rarer term and row 2 outranks rows 1 and 3.
    let in_a = search(&a, "personal", 50);
    assert!(
        position(&in_a, 2) < position(&in_a, 1),
        "store A order {in_a:?}"
    );
    assert!(
        position(&in_a, 2) < position(&in_a, 3),
        "store A order {in_a:?}"
    );
    // Store B: the three hidden rows put "gamma" in 4 of 11 rows,
    // making it the commoner term, so rows 1 and 3 now outrank row 2.
    let in_b = search(&b, "personal", 50);
    assert!(
        position(&in_b, 1) < position(&in_b, 2),
        "store B order {in_b:?}"
    );
    assert!(
        position(&in_b, 3) < position(&in_b, 2),
        "store B order {in_b:?}"
    );
    // At a cut of two, the rows inside the pool differ as well: this
    // is the observable the looser contract admits.
    assert!(search(&a, "personal", 2).contains(&2));
    assert!(!search(&b, "personal", 2).contains(&2));
    assert_eq!(sorted(search(&b, "personal", 2)), vec![1, 3]);
}

#[test]
fn partitioned_index_gives_identical_order_at_the_same_visible_cut() {
    for ceiling in ["shareable", "personal"] {
        let a = store(SHARED, Some(ceiling));
        let b = store(&store_b_rows(), Some(ceiling));
        for pool in [1, 2, 3, 50] {
            assert_eq!(
                search(&a, ceiling, pool),
                search(&b, ceiling, pool),
                "{ceiling} partition, pool {pool}"
            );
        }
        // The partition of store A is store A itself (it has no hidden
        // rows), so the strict order is the order store A already gives.
        assert_eq!(
            search(&a, ceiling, 50),
            search(&store(SHARED, None), ceiling, 50)
        );
    }
}

#[test]
fn full_trust_view_still_sees_hidden_rows() {
    let b = store(&store_b_rows(), None);
    assert_eq!(
        sorted(search(&b, "secret-bearing", 50)),
        vec![1, 2, 3, 9, 10, 11]
    );
}
