//! Parity tests against the production Python `atlas_search.py`.
//!
//! `fixtures/core-expectations.json` was recorded from the real
//! module by `fixtures/gen_core_fixtures.py`: search envelopes
//! over a miniature fixture corpus (LocalBackend, fixture query
//! vector), the Turso branch SQL byte-for-byte, and the
//! `turso_allowed_ids` / `turso_index_branch` call sequences
//! against a recording fake. atlas-core must reproduce all of it.
//!
//! The canary property of the fixture corpus: chunk 5
//! (secret-bearing) holds the query's exact vector and FTS terms,
//! chunk 6 (local-only) is nearly as close — both must be absent
//! at the default ceiling (filter-before-rank), and chunk 5 must
//! surface at vector rank 1 under a secret-bearing ceiling.
//!
//! Intentional deviation (code#5375, Gate B / code#5356): the Python
//! builds topics/projects filters as unescaped `LIKE ?` patterns, so
//! `%`/`_` act as wildcards and a `"` crosses JSON element boundaries.
//! atlas-core appends ` ESCAPE '\'` to those clauses and escapes the
//! pattern. For ordinary values (the recorded cases) the params are
//! identical; only the SQL text differs by that suffix, which
//! `assert_stmts_eq` normalizes away. The Python is deliberately left
//! unchanged (out of scope of #5375) and carries the same bug.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atlas_core::filters::{Filters, TURSO_SHAPE, branch_queries};
use atlas_core::turso::{PipelineTransport, TursoBackend};
use atlas_core::types::{Row, Stmt, Value};
use atlas_core::{
    DEFAULT_VEC_TABLE, FixedEmbedder, LocalBackend, SearchMode, SearchOptions, VectorBackend,
    run_get_chunk, run_search,
};
use serde_json::Value as Json;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn expectations() -> Json {
    let p = fixtures_dir().join("core-expectations.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("read expectations"))
        .expect("parse expectations")
}

fn fixture_spec() -> Json {
    let p = fixtures_dir().join("fixture_spec.json");
    serde_json::from_str(&std::fs::read_to_string(p).expect("read spec")).expect("parse spec")
}

/// Build the fixture corpus DB in a unique temp dir (the DB file
/// itself is never committed; the generator's SQL + spec are).
fn build_fixture_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-core-parity-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = dir.join("fixture.db");
    let _ = std::fs::remove_file(&db);
    let sql = std::fs::read_to_string(fixtures_dir().join("fixture_corpus.sql"))
        .expect("read corpus sql");
    let spec = fixture_spec();
    let con = rusqlite::Connection::open(&db).expect("open fixture db");
    con.execute_batch(&sql).expect("load corpus sql");
    let vectors = spec["vectors"].as_object().expect("spec vectors");
    for (cid, vec) in vectors {
        let mut blob = Vec::new();
        for x in vec.as_array().expect("vec array") {
            blob.extend_from_slice(&(x.as_f64().expect("f64") as f32).to_le_bytes());
        }
        con.execute(
            "INSERT INTO vec_snowflake_m_v2_768_f32 (chunk_id, vec) VALUES (?, ?)",
            rusqlite::params![cid.parse::<i64>().expect("cid"), blob],
        )
        .expect("insert vec");
    }
    drop(con);
    db
}

fn filters(
    kinds: &[&str],
    topics: &[&str],
    projects: &[&str],
    clients: &[&str],
    ceiling: &str,
    since: Option<&str>,
    until: Option<&str>,
) -> Filters {
    Filters::new(
        kinds.iter().map(|s| s.to_string()).collect(),
        topics.iter().map(|s| s.to_string()).collect(),
        projects.iter().map(|s| s.to_string()).collect(),
        clients.iter().map(|s| s.to_string()).collect(),
        ceiling,
        since.map(str::to_string),
        until.map(str::to_string),
    )
    .expect("filters")
}

fn default_filters() -> Filters {
    filters(&[], &[], &[], &[], "personal", None, None)
}

fn case_request(name: &str) -> (SearchOptions, Filters) {
    let mut o = SearchOptions::new("how do we back up the database");
    let f = match name {
        "hybrid_default" => default_filters(),
        "hybrid_secret_ceiling" => filters(&[], &[], &[], &[], "secret-bearing", None, None),
        "hybrid_shareable_ceiling" => filters(&[], &[], &[], &[], "shareable", None, None),
        "fts_default" => {
            o.mode = SearchMode::Fts;
            default_filters()
        }
        "vector_default" => {
            o.mode = SearchMode::Vector;
            default_filters()
        }
        "hybrid_kind_code" => filters(&["code"], &[], &[], &[], "personal", None, None),
        "hybrid_topic_nixos" => filters(&[], &["nixos"], &[], &[], "personal", None, None),
        "hybrid_client_codex_dates" => filters(
            &[],
            &[],
            &[],
            &["codex"],
            "personal",
            Some("2026-10-01"),
            Some("2026-10-31"),
        ),
        "hybrid_until_before_sB" => filters(
            &[],
            &[],
            &[],
            &["codex"],
            "personal",
            None,
            Some("2026-10-19"),
        ),
        "hybrid_grouped" => {
            o.group_by_session = true;
            o.k = 2;
            default_filters()
        }
        "hybrid_expand_message" => {
            o.expand = 1;
            o.with_message = true;
            o.k = 3;
            default_filters()
        }
        "hybrid_pool2_k2" => {
            o.pool = 2;
            o.k = 2;
            default_filters()
        }
        other => panic!("unknown case {other}"),
    };
    (o, f)
}

#[test]
fn local_search_envelopes_match_python() {
    let db = build_fixture_db("search");
    let be =
        LocalBackend::new(db.to_str().unwrap(), None, DEFAULT_VEC_TABLE, 0).expect("local backend");
    let exp = expectations();
    let qvec_raw: Vec<f32> = exp["qvec_raw"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect();
    let embedder = FixedEmbedder { raw: qvec_raw };
    for case in exp["search_cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let (opts, f) = case_request(name);
        let got =
            run_search(&be, &embedder, &f, &opts).unwrap_or_else(|e| panic!("case {name}: {e}"));
        assert_eq!(
            got,
            case["envelope"],
            "case {name} envelope mismatch\nrust: {}\npython: {}",
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&case["envelope"]).unwrap(),
        );
    }
}

#[test]
fn grouped_wrap_range_k_considers_all_candidates() {
    // #5423: k has no upper bound, and grouped_output's k * 10
    // consideration window wrapped for wrap-range k (release) or
    // panicked (debug). 5534023222112865485 * 10 mod 2^64 == 2, so
    // the pre-fix window collapsed to max(2, pool). A wrap-range k
    // must now behave exactly like a large sane k: every fused
    // candidate is considered.
    let db = build_fixture_db("wrapk");
    let be =
        LocalBackend::new(db.to_str().unwrap(), None, DEFAULT_VEC_TABLE, 0).expect("local backend");
    let exp = expectations();
    let qvec_raw: Vec<f32> = exp["qvec_raw"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_f64().unwrap() as f32)
        .collect();
    let embedder = FixedEmbedder { raw: qvec_raw };
    let run = |k: usize| {
        let mut o = SearchOptions::new("how do we back up the database");
        o.group_by_session = true;
        o.pool = 2;
        o.k = k;
        run_search(&be, &embedder, &default_filters(), &o).expect("grouped search")
    };
    let wrap = run(5_534_023_222_112_865_485);
    let sane = run(1_000_000);
    assert_eq!(wrap, sane, "wrap-range k must consider all candidates");
    let results = wrap["results"].as_array().expect("results array");
    assert!(!results.is_empty(), "grouped results must be non-empty");
}

#[test]
fn local_get_chunk_matches_python() {
    let db = build_fixture_db("getchunk");
    let be =
        LocalBackend::new(db.to_str().unwrap(), None, DEFAULT_VEC_TABLE, 0).expect("local backend");
    let exp = expectations();
    for case in exp["get_chunk_cases"].as_array().unwrap() {
        let (cid, neighbors) = match case["name"].as_str().unwrap() {
            "get_chunk_secret" => (5, 0),
            "get_chunk_neighbors" => (2, 1),
            other => panic!("unknown case {other}"),
        };
        let got = run_get_chunk(&be, cid, neighbors).expect("get chunk");
        assert_eq!(got, case["envelope"], "case {}", case["name"]);
    }
}

fn json_params_to_values(params: &Json) -> Vec<Value> {
    params
        .as_array()
        .unwrap()
        .iter()
        .map(|p| match p {
            Json::String(s) => Value::text(s.clone()),
            Json::Number(n) => n
                .as_i64()
                .map(Value::Integer)
                .unwrap_or_else(|| Value::Float(n.as_f64().unwrap())),
            Json::Null => Value::Null,
            other => panic!("unhandled fixture param {other}"),
        })
        .collect()
}

fn assert_stmts_eq(got: &[Stmt], recorded: &Json, ctx: &str) {
    let recorded = recorded.as_array().unwrap();
    assert_eq!(got.len(), recorded.len(), "{ctx}: statement count");
    for (i, (g, r)) in got.iter().zip(recorded.iter()).enumerate() {
        let sql = g.sql.replace(" LIKE ? ESCAPE '\\'", " LIKE ?");
        assert_eq!(sql, r["sql"].as_str().unwrap(), "{ctx}: stmt {i} sql");
        assert_eq!(
            g.args,
            json_params_to_values(&r["params"]),
            "{ctx}: stmt {i} params"
        );
    }
}

#[test]
fn turso_branch_queries_match_python() {
    let exp = expectations();
    let me = Some("\"back\" OR \"database\"");
    let qj = Some("[0.5, 0.25]");
    for case in exp["turso"]["branch_queries"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let (f, me_arg, qj_arg) = match name {
            "defaults" => (default_filters(), me, qj),
            "all_filters_secret" => (
                filters(
                    &["code"],
                    &["rust"],
                    &[],
                    &["codex"],
                    "secret-bearing",
                    Some("2026-10-01"),
                    Some("2026-10-31"),
                ),
                me,
                qj,
            ),
            "fts_only" => (default_filters(), me, None),
            "vector_only" => (default_filters(), None, qj),
            other => panic!("unknown case {other}"),
        };
        let got = branch_queries(TURSO_SHAPE, DEFAULT_VEC_TABLE, &f, me_arg, qj_arg, 100);
        assert_stmts_eq(&got, &case["stmts"], name);
    }
}

/// The recording fake from the generator, speaking the
/// PipelineTransport contract (one call may carry several
/// statements; the Python's fake execute saw them one by one, so
/// calls are recorded flattened, in order).
struct FakeTransport {
    calls: Mutex<Vec<Stmt>>,
}

impl FakeTransport {
    fn new() -> FakeTransport {
        FakeTransport {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn respond(&self, stmt: &Stmt) -> Vec<Row> {
        let sql = &stmt.sql;
        let row = |pairs: Vec<(&str, Value)>| {
            Row::new(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
        };
        if sql.contains("vector_top_k") {
            let depth = match stmt.args.get(2) {
                Some(Value::Integer(i)) => *i,
                other => panic!("top_k depth arg: {other:?}"),
            };
            let script: &[i64] = match depth {
                3 => &[5, 1, 6],
                6 => &[5, 1, 6, 2, 8, 3],
                other => panic!("unscripted depth {other}"),
            };
            return script
                .iter()
                .map(|i| row(vec![("id", Value::Integer(*i))]))
                .collect();
        }
        if sql.contains("vector_distance_cos") && sql.contains("chunk_id IN") {
            let dists: HashMap<i64, f64> = [(1, 0.01), (2, 0.3), (8, 0.07)].into_iter().collect();
            return stmt.args[1..]
                .iter()
                .map(|a| match a {
                    Value::Integer(cid) => row(vec![
                        ("cid", Value::Integer(*cid)),
                        ("dist", Value::Float(dists[cid])),
                    ]),
                    other => panic!("dist arg: {other:?}"),
                })
                .collect();
        }
        if sql.contains("c.sensitivity") {
            return [1, 2, 3, 4, 7, 8]
                .iter()
                .map(|i| row(vec![("cid", Value::Integer(*i))]))
                .collect();
        }
        if sql.contains("c2.id") {
            return [4, 7]
                .iter()
                .map(|i| row(vec![("cid", Value::Integer(*i))]))
                .collect();
        }
        panic!("unexpected SQL: {sql}");
    }

    fn recorded(&self) -> Json {
        let calls = self.calls.lock().unwrap();
        Json::Array(
            calls
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "sql": s.sql,
                        "params": s.args.iter().map(|a| match a {
                            Value::Text(t) => Json::String(t.clone()),
                            Value::Integer(i) => Json::from(*i),
                            Value::Float(f) => Json::from(*f),
                            Value::Null => Json::Null,
                            Value::Blob(_) => panic!("blob param in turso fixture"),
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect(),
        )
    }
}

impl PipelineTransport for FakeTransport {
    fn pipeline(&self, stmts: &[Stmt]) -> atlas_core::Result<Vec<Vec<Row>>> {
        let mut out = Vec::new();
        for s in stmts {
            self.calls.lock().unwrap().push(s.clone());
            out.push(self.respond(s));
        }
        Ok(out)
    }
}

#[test]
fn turso_allowed_ids_and_index_branches_match_python() {
    let exp = expectations();
    let turso = &exp["turso"];

    // allowed-ids, no session filters: one statement.
    let fake = FakeTransport::new();
    let be = TursoBackend::with_transport(fake, DEFAULT_VEC_TABLE);
    let mut got: Vec<i64> = be
        .allowed_ids(&default_filters())
        .expect("allowed ids")
        .into_iter()
        .collect();
    got.sort_unstable();
    assert_eq!(Json::from(got), turso["allowed_ids_plain"]["result"]);
    assert_eq!(
        be.transport().recorded(),
        turso["allowed_ids_plain"]["calls"]
    );

    // allowed-ids with a kind filter: sensitivity set ∩ session set.
    let fake = FakeTransport::new();
    let be = TursoBackend::with_transport(fake, DEFAULT_VEC_TABLE);
    let f = filters(&["code"], &[], &[], &[], "personal", None, None);
    let mut got: Vec<i64> = be
        .allowed_ids(&f)
        .expect("allowed ids")
        .into_iter()
        .collect();
    got.sort_unstable();
    assert_eq!(Json::from(got), turso["allowed_ids_kind"]["result"]);
    assert_eq!(
        be.transport().recorded(),
        turso["allowed_ids_kind"]["calls"]
    );

    // index fetch-to-fill at pool 3: depths 3 then 6, survivors
    // [1, 2, 8] in index order, exact distances fetched after.
    let fake = FakeTransport::new();
    let be = TursoBackend::with_transport(fake, DEFAULT_VEC_TABLE);
    let got = be
        .index_branch(
            &default_filters(),
            "[0.5, 0.25]",
            3,
            VectorBackend::IndexFill,
        )
        .expect("index fill");
    let got_json = Json::Array(
        got.iter()
            .map(|h| serde_json::json!([h.chunk_id, h.dist]))
            .collect(),
    );
    assert_eq!(got_json, turso["index_fill"]["result"]);
    assert_eq!(be.transport().recorded(), turso["index_fill"]["calls"]);

    // index fixed-fetch: one depth-3 fetch, survivors [1].
    let fake = FakeTransport::new();
    let be = TursoBackend::with_transport(fake, DEFAULT_VEC_TABLE);
    let got = be
        .index_branch(
            &default_filters(),
            "[0.5, 0.25]",
            3,
            VectorBackend::IndexFixed,
        )
        .expect("index fixed");
    let got_json = Json::Array(
        got.iter()
            .map(|h| serde_json::json!([h.chunk_id, h.dist]))
            .collect(),
    );
    assert_eq!(got_json, turso["index_fixed"]["result"]);
    assert_eq!(be.transport().recorded(), turso["index_fixed"]["calls"]);
}
