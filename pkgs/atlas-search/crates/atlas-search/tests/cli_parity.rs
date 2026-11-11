//! CLI parity: the step-2 fixture corpus exercised through the
//! compiled `atlas-search` binary (developing-today/code#5353).
//!
//! For every search/get-chunk case recorded in
//! `fixtures/core-expectations.json` (envelopes produced by the
//! production Python), the binary runs against the fixture DB
//! with `--backend local` and the harness query-vector hook
//! (`ATLAS_SEARCH_QVEC_FILE` — the same monkeypatch the Python
//! fixture generator applies), and its stdout must parse to the
//! recorded envelope exactly. `--text` goldens (rendered by the
//! Python's own `render_text` from those envelopes) live in
//! `tests/golden/` and are compared byte-for-byte.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Build the fixture corpus DB in a unique temp dir (the same
/// builder the atlas-core parity tests use; the DB is never
/// committed).
fn build_fixture_db(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-cli-parity-{}-{tag}", std::process::id()));
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

fn qvec_file() -> PathBuf {
    // Written once per test-binary run. The path is named by pid and
    // shared by every test in this binary, and the tests run in
    // parallel: rewriting the file on every call let one test's CLI
    // read it mid-truncate from another test's rewrite and fail with
    // an empty-JSON error (seen under `cargo test --all-targets`,
    // where the extra load widens the race window). The content is
    // the shared golden fixture, identical for every caller.
    static QVEC: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    QVEC.get_or_init(|| {
        let exp = expectations();
        let p = std::env::temp_dir().join(format!("atlas-cli-qvec-{}.json", std::process::id()));
        std::fs::write(&p, serde_json::to_string(&exp["qvec_raw"]).unwrap()).expect("write qvec");
        p
    })
    .clone()
}

fn run_bin(db: &Path, args: &[&str]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_atlas-search");
    Command::new(bin)
        .args(args)
        .arg("--db")
        .arg(db)
        .env("ATLAS_SEARCH_QVEC_FILE", qvec_file())
        .env_remove("ATLAS_VEC_DB")
        .env_remove("ATLAS_SEARCH_VIA")
        .env_remove("ATLAS_SEARCH_URL")
        .env_remove("ATLAS_SEARCH_MODEL")
        .env_remove("ATLAS_VEC_TABLE")
        .output()
        .expect("run atlas-search")
}

/// Case name -> argv tail (after the subcommand), mirroring the
/// atlas-core parity test's `case_request`.
fn search_tail(name: &str) -> Vec<&'static str> {
    match name {
        "hybrid_default" => vec![],
        "hybrid_secret_ceiling" => vec!["--max-sensitivity", "secret-bearing"],
        "hybrid_shareable_ceiling" => vec!["--max-sensitivity", "shareable"],
        "fts_default" => vec!["--mode", "fts"],
        "vector_default" => vec!["--mode", "vector"],
        "hybrid_kind_code" => vec!["--kind", "code"],
        "hybrid_topic_nixos" => vec!["--topic", "nixos"],
        "hybrid_client_codex_dates" => vec![
            "--client",
            "codex",
            "--since",
            "2026-10-01",
            "--until",
            "2026-10-31",
        ],
        "hybrid_until_before_sB" => vec!["--client", "codex", "--until", "2026-10-19"],
        "hybrid_grouped" => vec!["--group-by-session", "--k", "2"],
        "hybrid_expand_message" => vec!["--expand", "1", "--with-message", "--k", "3"],
        "hybrid_pool2_k2" => vec!["--pool", "2", "--k", "2"],
        other => panic!("unknown case {other}"),
    }
}

#[test]
fn cli_search_envelopes_match_python() {
    let db = build_fixture_db("search");
    let exp = expectations();
    for case in exp["search_cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut args = vec![
            "search",
            "how do we back up the database",
            "--backend",
            "local",
        ];
        args.extend(search_tail(name));
        let out = run_bin(&db, &args);
        assert!(
            out.status.success(),
            "case {name}: exit {:?}, stderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Json = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("case {name}: stdout not JSON: {e}"));
        assert_eq!(
            got,
            case["envelope"],
            "case {name} envelope mismatch\ncli: {}\npython: {}",
            serde_json::to_string_pretty(&got).unwrap(),
            serde_json::to_string_pretty(&case["envelope"]).unwrap(),
        );
    }
}

#[test]
fn cli_rejects_expand_above_limit_before_backend_work() {
    let db = std::env::temp_dir()
        .join(format!("atlas-cli-parity-{}-missing", std::process::id()))
        .join("missing.db");
    let out = run_bin(
        &db,
        &["search", "query", "--backend", "local", "--expand", "101"],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--expand must be between 0 and 100"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("missing.db"),
        "backend was reached: {stderr}"
    );
}

#[test]
fn cli_rejects_unsupported_query_controls_before_backend_work() {
    let db = std::env::temp_dir()
        .join(format!(
            "atlas-cli-parity-{}-unsupported",
            std::process::id()
        ))
        .join("missing.db");
    for (flags, expected) in [
        (
            vec!["--jaccard_depth", "5"],
            "Jaccard controls are not implemented",
        ),
        (
            vec!["--planner_enabled"],
            "planner/spell stages are not implemented",
        ),
        (
            vec!["--raw", "--expand", "1"],
            "raw mode cannot be combined",
        ),
    ] {
        let mut args = vec!["search", "query", "--backend", "local"];
        args.extend(flags);
        let out = run_bin(&db, &args);
        assert_eq!(out.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(
            !stderr.contains("missing.db"),
            "backend was reached: {stderr}"
        );
    }
}

#[test]
fn cli_wires_rrf_and_signal_weights() {
    let db = build_fixture_db("weighted-rrf");
    let out = run_bin(
        &db,
        &[
            "search",
            "how do we back up the database",
            "--backend",
            "local",
            "--rrf_k",
            "20",
            "--fts_weight",
            "2",
            "--vector_weight",
            "1",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got: Json = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(got["results"][0]["score"], 0.142857);
}

#[test]
fn raw_mode_does_not_widen_the_sensitivity_ceiling() {
    let db = build_fixture_db("raw-ceiling");
    let normal = run_bin(
        &db,
        &[
            "search",
            "how do we back up the database",
            "--backend",
            "local",
            "--max-sensitivity",
            "shareable",
        ],
    );
    let raw = run_bin(
        &db,
        &[
            "search",
            "how do we back up the database",
            "--backend",
            "local",
            "--max-sensitivity",
            "shareable",
            "--raw",
        ],
    );
    assert!(normal.status.success());
    assert!(raw.status.success());
    let normal: Json = serde_json::from_slice(&normal.stdout).unwrap();
    let raw: Json = serde_json::from_slice(&raw.stdout).unwrap();
    assert_eq!(raw, normal, "raw mode must keep the same filters/ceiling");
}

#[test]
fn cli_get_chunk_matches_python() {
    let db = build_fixture_db("getchunk");
    let exp = expectations();
    for case in exp["get_chunk_cases"].as_array().unwrap() {
        let (cid, neighbors) = match case["name"].as_str().unwrap() {
            "get_chunk_secret" => ("5", "0"),
            "get_chunk_neighbors" => ("2", "1"),
            other => panic!("unknown case {other}"),
        };
        let out = run_bin(
            &db,
            &[
                "get-chunk",
                cid,
                "--neighbors",
                neighbors,
                "--backend",
                "local",
            ],
        );
        assert!(
            out.status.success(),
            "case {}: exit {:?}, stderr: {}",
            case["name"],
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let got: Json = serde_json::from_slice(&out.stdout).expect("stdout JSON");
        assert_eq!(got, case["envelope"], "case {}", case["name"]);
    }
}

#[test]
fn cli_get_chunk_not_found_exits_1() {
    let db = build_fixture_db("notfound");
    let out = run_bin(&db, &["get-chunk", "999", "--backend", "local"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "atlas-search: chunk 999 not found"
    );
}

#[test]
fn cli_get_chunk_neighbors_above_cap_exits_2() {
    // Command-level coverage for the --neighbors cap (#6594): the unit
    // tests in main.rs call count() directly, so dropping the cap at
    // the cmd_get_chunk call site would fail nothing there. Through
    // the binary, 101 must be refused before any lookup, with exit 2
    // and the bound message on stderr.
    let db = build_fixture_db("neighborscap");
    let out = run_bin(
        &db,
        &["get-chunk", "2", "--neighbors", "101", "--backend", "local"],
    );
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--neighbors must be between 0 and 100"),
        "stderr: {stderr}"
    );
}

#[test]
fn cli_text_goldens_match_python() {
    let db = build_fixture_db("text");
    let cases: [(&str, &str); 3] = [
        ("hybrid_default", "text_search_flat.txt"),
        ("hybrid_grouped", "text_search_grouped.txt"),
        ("get_chunk_neighbors", "text_get_chunk.txt"),
    ];
    let golden_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    for (name, file) in cases {
        let golden = std::fs::read_to_string(golden_dir.join(file))
            .unwrap_or_else(|e| panic!("read golden {file}: {e}"));
        let out = if name == "get_chunk_neighbors" {
            run_bin(
                &db,
                &[
                    "get-chunk",
                    "2",
                    "--neighbors",
                    "1",
                    "--backend",
                    "local",
                    "--text",
                ],
            )
        } else {
            let mut args = vec![
                "search",
                "how do we back up the database",
                "--backend",
                "local",
                "--text",
            ];
            args.extend(search_tail(name));
            run_bin(&db, &args)
        };
        assert!(
            out.status.success(),
            "golden {name}: exit {:?}, stderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            golden,
            "golden {file} mismatch"
        );
    }
}

// ------------------------------------------------ shard fan-out ----------
//
// Issue #5755 slice 1: `search --shards dir:<path>` serves the vector
// branch from atlas-core shard files while FTS/details stay on the
// (monolith) backend. Oracle: the recorded production-Python
// envelopes — with the shard fan-out the result lists must be
// identical (only `notes` gains the fan-out note).

/// Split the fixture corpus into tiered core shards: shareable ->
/// public, personal/local-only -> internal (two shards), secret-bearing
/// -> secret. Chunk ids are preserved, as in the restored core.
fn build_shard_dir(db: &Path, tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atlas-cli-shards-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("shard dir");
    let src = rusqlite::Connection::open(db).expect("open fixture");
    let plan: &[(&str, &[i64])] = &[
        ("atlas-core-internal-work-01", &[1, 2, 3]),
        ("atlas-core-internal-misc-01", &[6, 8]),
        ("atlas-core-public-docs-01", &[4, 7]),
        ("atlas-core-secret-main-01", &[5]),
    ];
    for (name, ids) in plan {
        let rest = name.strip_prefix("atlas-core-").unwrap();
        let (tier, rest) = rest.split_once('-').unwrap();
        let (topic, part) = rest.rsplit_once('-').unwrap();
        let con = rusqlite::Connection::open(dir.join(format!("{name}.db"))).expect("shard");
        con.execute_batch(
            "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
               chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
               sensitivity TEXT);
             CREATE TABLE vec_snowflake_m_v2_768_f32 (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
             CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        for id in *ids {
            let row: (String, i64, String, String, String, Vec<u8>) = src
                .query_row(
                    "SELECT c.message_id, c.chunk_idx, c.text, c.content_hash, c.sensitivity, v.vec \
                     FROM chunks c JOIN vec_snowflake_m_v2_768_f32 v ON v.chunk_id = c.id WHERE c.id = ?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .unwrap();
            con.execute(
                "INSERT INTO chunks VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![id, row.0, row.1, row.2, row.3, row.4],
            )
            .unwrap();
            con.execute(
                "INSERT INTO vec_snowflake_m_v2_768_f32 VALUES (?, ?)",
                rusqlite::params![id, row.5],
            )
            .unwrap();
        }
        for (k, v) in [
            ("shard", name.to_string()),
            ("tier", tier.to_string()),
            ("topic", topic.to_string()),
            ("part", part.trim_start_matches('0').to_string()),
            ("expected_chunks", ids.len().to_string()),
            ("metric", "cosine".to_string()),
            ("embedding_model", "snowflake-arctic-embed-m-v2.0".to_string()),
        ] {
            con.execute(
                "INSERT INTO atlas_manifest VALUES (?, ?)",
                rusqlite::params![k, v],
            )
            .unwrap();
        }
    }
    dir
}

#[test]
fn cli_shard_fanout_matches_python_vector_results() {
    let db = build_fixture_db("shards");
    let shards = build_shard_dir(&db, "ok");
    let spec = format!("dir:{}", shards.display());
    let exp = expectations();
    for name in [
        "vector_default",
        "hybrid_default",
        "hybrid_secret_ceiling",
        "hybrid_shareable_ceiling",
        "hybrid_pool2_k2",
    ] {
        let case = exp["search_cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("no case {name}"));
        let mut args = vec![
            "search",
            "how do we back up the database",
            "--backend",
            "local",
            "--shards",
            &spec,
        ];
        args.extend(search_tail(name));
        let out = run_bin(&db, &args);
        assert!(
            out.status.success(),
            "case {name}: exit {:?}, stderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let mut got: Json = serde_json::from_slice(&out.stdout).expect("stdout JSON");
        let notes = got
            .as_object_mut()
            .unwrap()
            .remove("notes")
            .unwrap_or_default();
        assert!(
            notes.to_string().contains("shard fan-out"),
            "case {name}: missing fan-out note in {notes}"
        );
        let mut want = case["envelope"].clone();
        want.as_object_mut().unwrap().remove("notes");
        assert_eq!(
            got,
            want,
            "case {name} differs from the Python envelope under shard fan-out\ncli: {}",
            serde_json::to_string_pretty(&got).unwrap()
        );
    }
}

#[test]
fn cli_shard_fanout_default_never_touches_secret_shard() {
    let db = build_fixture_db("shards-ceiling");
    let shards = build_shard_dir(&db, "ceiling");
    // Corrupt the secret shard so ANY open of it would fail loudly: the
    // default ceiling must not even try.
    std::fs::write(
        shards.join("atlas-core-secret-main-01.db"),
        b"not a database",
    )
    .unwrap();
    let spec = format!("dir:{}", shards.display());
    let out = run_bin(
        &db,
        &[
            "search",
            "backup",
            "--backend",
            "local",
            "--mode",
            "vector",
            "--shards",
            &spec,
        ],
    );
    assert!(
        out.status.success(),
        "default ceiling opened the secret shard? stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // ...whereas an explicit secret-bearing ceiling does open it (and fails here).
    let out = run_bin(
        &db,
        &[
            "search",
            "backup",
            "--backend",
            "local",
            "--mode",
            "vector",
            "--shards",
            &spec,
            "--max-sensitivity",
            "secret-bearing",
        ],
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("atlas-core-secret-main-01"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
