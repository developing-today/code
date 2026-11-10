//! Shard fan-out tests (developing-today/code#5755, slice 1) — no
//! live Turso, no secrets: shards are local SQLite files whose
//! connections carry `vector32` / `vector_distance_cos`, and the
//! statements are the very ones sent to a hosted shard.
//!
//! Oracles (the testing doctrine asks each test to name one):
//! * fan-out == one store holding the union of all shards, scanned
//!   with the identical statement (the monolith exact-scan oracle,
//!   same shape as the #5669 restore parity run);
//! * the connect log of a recording connector (physical ceiling:
//!   a tier above the ceiling is never connected);
//! * hand-computed orderings from planted vectors.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use atlas_core::backend::SearchBackend;
use atlas_core::filters::{Filters, turso_vector_scan_stmt};
use atlas_core::shards::{
    DirLister, DiscoveryOptions, FanoutQuery, LocalShardTransport, ShardConnector, ShardEndpoint,
    ShardLocation, ShardSet, ShardTransport, ShardedBackend, StaticLister, Tier, TierCeiling,
    discover, fanout_scan,
};
use atlas_core::turso::{PipelineTransport, qvec_json};
use atlas_core::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, DEFAULT_VEC_TABLE, MessageRow, NeighborRow,
    Row, Stmt, VectorBackend,
};
use atlas_core::{Error, Result};

const DIM: usize = 8;

// ------------------------------------------------------------ rng ------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) as f32 * 2.0 - 1.0
    }
    fn vec(&mut self) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM).map(|_| self.unit()).collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

// ----------------------------------------------------------- fixtures ---

struct Chunk {
    id: i64,
    sens: &'static str,
    vec: Vec<f32>,
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-shards-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn create_store(path: &Path, chunks: &[Chunk]) -> rusqlite::Connection {
    let _ = std::fs::remove_file(path);
    let con = rusqlite::Connection::open(path).expect("create store");
    con.execute_batch(&format!(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
         CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);"
    ))
    .expect("schema");
    con.execute_batch("BEGIN").expect("begin");
    for c in chunks {
        con.execute(
            "INSERT INTO chunks VALUES (?, ?, 0, ?, ?, ?)",
            rusqlite::params![
                c.id,
                format!("m{}", c.id),
                "t",
                format!("h{}", c.id),
                c.sens
            ],
        )
        .expect("chunk");
        con.execute(
            &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?, ?)"),
            rusqlite::params![c.id, blob(&c.vec)],
        )
        .expect("vec");
    }
    con.execute_batch("COMMIT").expect("commit");
    con
}

fn manifest(con: &rusqlite::Connection, pairs: &[(&str, &str)]) {
    for (k, v) in pairs {
        con.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES (?, ?)",
            rusqlite::params![k, v],
        )
        .expect("manifest");
    }
}

/// Write `<dir>/<name>.db` as a well-formed shard and return its rows.
fn write_shard(dir: &Path, name: &str, chunks: &[Chunk]) {
    let (tier, topic, part) = atlas_core::shards::parse_shard_name(name).expect("shard name");
    let con = create_store(&dir.join(format!("{name}.db")), chunks);
    manifest(
        &con,
        &[
            ("shard", name),
            ("tier", tier.name()),
            ("topic", &topic),
            ("part", &part.to_string()),
            ("expected_chunks", &chunks.len().to_string()),
            ("metric", "cosine"),
            ("engine", "tursodb"),
        ],
    );
}

fn filters(ceiling: &str) -> Filters {
    Filters::new(vec![], vec![], vec![], vec![], ceiling, None, None).expect("filters")
}

/// Open connector with a connect log; `closed` tiers have no credential.
struct LogConnector {
    closed: Vec<Tier>,
    connects: Mutex<Vec<String>>,
    scan_in_flight: Arc<AtomicUsize>,
    scan_max: Arc<AtomicUsize>,
    scan_delay: Duration,
}

impl LogConnector {
    fn new(closed: Vec<Tier>) -> LogConnector {
        LogConnector {
            closed,
            connects: Mutex::new(Vec::new()),
            scan_in_flight: Arc::new(AtomicUsize::new(0)),
            scan_max: Arc::new(AtomicUsize::new(0)),
            scan_delay: Duration::ZERO,
        }
    }
    fn connected(&self) -> Vec<String> {
        let mut v = self.connects.lock().unwrap().clone();
        v.sort();
        v
    }
}

/// Counts concurrent scan statements (those using the distance fn).
struct Gauge {
    inner: LocalShardTransport,
    in_flight: Arc<AtomicUsize>,
    max: Arc<AtomicUsize>,
    delay: Duration,
}
impl PipelineTransport for Gauge {
    fn pipeline(&self, stmts: &[Stmt]) -> Result<Vec<Vec<Row>>> {
        let scan = stmts.iter().any(|s| s.sql.contains("vector_distance_cos"));
        if scan {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(self.delay);
        }
        let out = self.inner.pipeline(stmts);
        if scan {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
        out
    }
}

impl ShardConnector for LogConnector {
    fn can_open(&self, tier: Tier) -> bool {
        !self.closed.contains(&tier)
    }
    fn connect(&self, ep: &ShardEndpoint, _tier: Tier) -> Result<Arc<dyn ShardTransport>> {
        self.connects.lock().unwrap().push(ep.name.clone());
        let ShardLocation::File(p) = &ep.location else {
            panic!("tests use files");
        };
        Ok(Arc::new(Gauge {
            inner: LocalShardTransport::open(p)?,
            in_flight: self.scan_in_flight.clone(),
            max: self.scan_max.clone(),
            delay: self.scan_delay,
        }))
    }
}

/// A corpus spread over shards, plus the union store (the oracle).
struct Corpus {
    dir: PathBuf,
    oracle: LocalShardTransport,
    all: Vec<(String, Vec<Chunk>)>,
}

/// Internal shards (several topics), one public, one secret, plus a
/// content store and a stray file that are not scan shards. Chunk
/// ids are globally unique, as in the restored core.
fn corpus(tag: &str, seed: u64) -> Corpus {
    let dir = temp_dir(tag);
    let mut rng = Rng(seed);
    let mut next_id = 1i64;
    let plan: &[(&str, usize)] = &[
        ("atlas-core-internal-work-01", 30),
        ("atlas-core-internal-work-02", 30),
        ("atlas-core-internal-media-01", 25),
        ("atlas-core-internal-misc-01", 10),
        ("atlas-core-public-docs-01", 12),
        ("atlas-core-secret-main-01", 9),
    ];
    let mut all = Vec::new();
    for (name, n) in plan {
        let mut chunks = Vec::new();
        for i in 0..*n {
            let sens = match (name.contains("-secret-"), name.contains("-public-"), i % 7) {
                (true, _, _) => "secret-bearing",
                (_, true, _) => "shareable",
                (_, _, 3) => "local-only",
                _ => "personal",
            };
            chunks.push(Chunk {
                id: next_id,
                sens,
                vec: rng.vec(),
            });
            next_id += 1;
        }
        write_shard(&dir, name, &chunks);
        all.push((name.to_string(), chunks));
    }
    // Not scan shards: a content store and an unrelated database.
    create_store(&dir.join("atlas-core-internal-content.db"), &[]);
    create_store(&dir.join("atlas.db"), &[]);
    let union: Vec<Chunk> = all
        .iter()
        .flat_map(|(_, cs)| {
            cs.iter().map(|c| Chunk {
                id: c.id,
                sens: c.sens,
                vec: c.vec.clone(),
            })
        })
        .collect();
    let opath = dir.join("oracle.sqlite");
    drop(create_store(&opath, &union));
    Corpus {
        dir,
        oracle: LocalShardTransport::open(&opath).expect("oracle"),
        all,
    }
}

impl Corpus {
    fn discover(&self, conn: &LogConnector, ceiling: Tier) -> Result<ShardSet> {
        discover(
            &DirLister {
                dir: self.dir.clone(),
            },
            conn,
            &DiscoveryOptions::new(TierCeiling(ceiling)),
        )
    }

    /// The oracle: the identical statement over the union store.
    fn oracle_top(&self, f: &Filters, q: &[f32], k: usize) -> Vec<(i64, f64)> {
        let stmt = turso_vector_scan_stmt(DEFAULT_VEC_TABLE, f, &qvec_json(q), k);
        self.oracle.pipeline(&[stmt]).unwrap()[0]
            .iter()
            .map(|r| (r.get_i64("cid").unwrap(), r.get_f64("dist").unwrap()))
            .collect()
    }
}

// -------------------------------------------------------------- tests ---

#[test]
fn discovery_resolves_shards_from_manifests_internal_first() {
    let c = corpus("discover", 11);
    let conn = LogConnector::new(vec![]);
    let set = c.discover(&conn, Tier::Secret).unwrap();
    let names: Vec<&str> = set.shards().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            // internal (topic, part order), then public, then secret
            "atlas-core-internal-media-01",
            "atlas-core-internal-misc-01",
            "atlas-core-internal-work-01",
            "atlas-core-internal-work-02",
            "atlas-core-public-docs-01",
            "atlas-core-secret-main-01",
        ]
    );
    let work1 = &set.shards()[2];
    assert_eq!(
        (work1.tier, work1.topic.as_str(), work1.part),
        (Tier::Internal, "work", 1)
    );
    assert_eq!((work1.chunks, work1.expected_chunks), (30, Some(30)));
    // The content store is named but not a scan shard; the stray DB is ignored.
    assert!(
        set.rejections
            .iter()
            .any(|r| r.name == "atlas-core-internal-content")
    );
    assert!(
        !conn
            .connected()
            .iter()
            .any(|n| n.contains("content") || n == "atlas")
    );
}

#[test]
fn new_shard_joins_without_code_changes_and_moves_the_snapshot_id() {
    let c = corpus("join", 12);
    let before = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    let id_before = before.snapshot_id();
    assert_eq!(before.snapshot_id(), id_before, "snapshot id is stable");

    // A brand-new topic shard appears on disk: next discovery has it.
    write_shard(
        &c.dir,
        "atlas-core-internal-recipes-01",
        &[Chunk {
            id: 9001,
            sens: "personal",
            vec: Rng(5).vec(),
        }],
    );
    let after = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    assert_eq!(after.shards().len(), before.shards().len() + 1);
    assert!(after.shards().iter().any(|s| s.topic == "recipes"));
    assert_ne!(after.snapshot_id(), id_before, "re-shard invalidates");

    // A row added to an existing shard (a promotion batch) changes counts.
    let con = rusqlite::Connection::open(c.dir.join("atlas-core-internal-recipes-01.db")).unwrap();
    con.execute(
        "INSERT INTO chunks VALUES (9002, 'm', 0, 't', 'h9002', 'personal')",
        [],
    )
    .unwrap();
    let grown = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    assert_ne!(
        grown.snapshot_id(),
        after.snapshot_id(),
        "per-shard counts matter"
    );
}

#[test]
fn integrity_failures_abort_unless_lenient() {
    let c = corpus("integrity", 13);
    // Manifest says secret, name says internal: never trusted.
    let con = rusqlite::Connection::open(c.dir.join("atlas-core-internal-misc-01.db")).unwrap();
    con.execute(
        "UPDATE atlas_manifest SET value = 'secret' WHERE \"key\" = 'tier'",
        [],
    )
    .unwrap();
    let err = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .err()
        .unwrap();
    assert!(err.to_string().contains("disagrees with its name"), "{err}");

    let lenient = discover(
        &DirLister { dir: c.dir.clone() },
        &LogConnector::new(vec![]),
        &DiscoveryOptions {
            lenient: true,
            ..DiscoveryOptions::new(TierCeiling(Tier::Internal))
        },
    )
    .unwrap();
    assert!(
        lenient
            .shards()
            .iter()
            .all(|s| s.name != "atlas-core-internal-misc-01")
    );
    assert!(
        lenient
            .rejections
            .iter()
            .any(|r| r.name == "atlas-core-internal-misc-01")
    );

    // A non-cosine shard must not be scanned with cosine.
    let con = rusqlite::Connection::open(c.dir.join("atlas-core-internal-misc-01.db")).unwrap();
    con.execute(
        "UPDATE atlas_manifest SET value = 'internal' WHERE \"key\" = 'tier'",
        [],
    )
    .unwrap();
    con.execute(
        "UPDATE atlas_manifest SET value = 'l2' WHERE \"key\" = 'metric'",
        [],
    )
    .unwrap();
    let err = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .err()
        .unwrap();
    assert!(err.to_string().contains("not cosine"), "{err}");
}

#[test]
fn fanout_equals_monolith_oracle_over_random_queries() {
    let c = corpus("oracle", 21);
    let set = c
        .discover(&LogConnector::new(vec![]), Tier::Secret)
        .unwrap();
    let mut rng = Rng(0x5eed);
    for round in 0..25 {
        let q = rng.vec();
        let k = 1 + (rng.next() % 40) as usize;
        for ceiling in ["shareable", "personal", "local-only", "secret-bearing"] {
            let f = filters(ceiling);
            let got = fanout_scan(
                &set,
                &FanoutQuery {
                    qvec: &q,
                    shard_top_k: 40,
                    global_top_k: k,
                    filters: &f,
                    topics: None,
                    ceiling: TierCeiling::from_sensitivity(f.max_sensitivity),
                },
            )
            .unwrap();
            let got: Vec<(i64, f64)> = got.hits.iter().map(|h| (h.chunk_id, h.dist)).collect();
            let want = c.oracle_top(&f, &q, k);
            // Same ids, same order, same distances (ties are measure-zero
            // with random vectors; the merge tie-break is unit-tested).
            assert_eq!(got.len(), want.len(), "round {round} {ceiling} k={k}");
            for (g, w) in got.iter().zip(&want) {
                assert_eq!(g.0, w.0, "round {round} {ceiling} k={k}");
                assert!((g.1 - w.1).abs() < 1e-12, "round {round} {ceiling}");
            }
        }
    }
}

#[test]
fn physical_ceiling_never_connects_to_tiers_above() {
    let c = corpus("ceiling", 31);
    // Plant the query as an exact copy of a secret chunk.
    let secret = &c.all.iter().find(|(n, _)| n.contains("secret")).unwrap().1[0];
    let (q, sid) = (secret.vec.clone(), secret.id);

    // Service opened at the internal ceiling: secret + (no-cred) tiers untouched.
    let conn = LogConnector::new(vec![Tier::Secret]);
    let set = c.discover(&conn, Tier::Internal).unwrap();
    assert!(
        conn.connected().iter().all(|n| !n.contains("secret")),
        "{:?}",
        conn.connected()
    );
    assert!(
        set.rejections
            .iter()
            .any(|r| r.name == "atlas-core-secret-main-01")
    );
    let f = filters("personal");
    let res = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 100,
            global_top_k: 100,
            filters: &f,
            topics: None,
            ceiling: TierCeiling::from_sensitivity(f.max_sensitivity),
        },
    )
    .unwrap();
    assert!(
        res.hits
            .iter()
            .all(|h| h.chunk_id != sid && h.tier != Tier::Secret)
    );
    assert!(res.scanned.iter().all(|n| !n.contains("secret")));

    // The same internal-only set refuses a secret-bearing caller outright.
    let f = filters("secret-bearing");
    let err = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 5,
            global_top_k: 5,
            filters: &f,
            topics: None,
            ceiling: TierCeiling::from_sensitivity(f.max_sensitivity),
        },
    )
    .err()
    .unwrap();
    assert!(
        err.to_string().contains("secret-capable")
            || err.to_string().contains("secret-capable credential"),
        "{err}"
    );

    // Ceiling at secret but no secret credential: discovery fails closed.
    let err = c
        .discover(&LogConnector::new(vec![Tier::Secret]), Tier::Secret)
        .err()
        .unwrap();
    assert!(err.to_string().contains("secret-tier credential"), "{err}");

    // Secret-capable consumer + explicit secret ceiling: exact match is rank 1.
    let set = c
        .discover(&LogConnector::new(vec![]), Tier::Secret)
        .unwrap();
    let res = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 3,
            global_top_k: 3,
            filters: &f,
            topics: None,
            ceiling: TierCeiling(Tier::Secret),
        },
    )
    .unwrap();
    assert_eq!(res.hits[0].chunk_id, sid);
    assert_eq!(res.hits[0].tier, Tier::Secret);
    assert!(res.hits[0].dist.abs() < 1e-6);

    // Default (personal) caller on the secret-capable set still never scans secret.
    let f = filters("personal");
    let res = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 100,
            global_top_k: 100,
            filters: &f,
            topics: None,
            ceiling: TierCeiling::from_sensitivity(f.max_sensitivity),
        },
    )
    .unwrap();
    assert!(res.scanned.iter().all(|n| !n.contains("secret")));
    assert!(res.hits.iter().all(|h| h.chunk_id != sid));

    // Shareable caller sees only the public tier.
    let f = filters("shareable");
    let res = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 100,
            global_top_k: 100,
            filters: &f,
            topics: None,
            ceiling: TierCeiling::from_sensitivity(f.max_sensitivity),
        },
    )
    .unwrap();
    assert_eq!(res.scanned, ["atlas-core-public-docs-01"]);
}

#[test]
fn topic_scope_touches_only_that_topics_shards() {
    let c = corpus("topic", 41);
    let set = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    let f = filters("personal");
    let q = Rng(7).vec();
    let topics = vec!["work".to_string()];
    let res = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 10,
            global_top_k: 10,
            filters: &f,
            topics: Some(&topics),
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .unwrap();
    assert_eq!(
        res.scanned,
        ["atlas-core-internal-work-01", "atlas-core-internal-work-02"]
    );
    assert!(res.hits.iter().all(|h| h.shard.contains("-work-")));
}

#[test]
fn scans_run_in_parallel() {
    let c = corpus("parallel", 51);
    let mut conn = LogConnector::new(vec![]);
    conn.scan_delay = Duration::from_millis(120);
    let set = c.discover(&conn, Tier::Internal).unwrap();
    let f = filters("personal");
    let q = Rng(9).vec();
    let started = std::time::Instant::now();
    fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &q,
            shard_top_k: 10,
            global_top_k: 10,
            filters: &f,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .unwrap();
    let n = set.shards().len();
    assert_eq!(n, 5);
    assert_eq!(
        conn.scan_max.load(Ordering::SeqCst),
        n,
        "all shards in flight together"
    );
    assert!(
        started.elapsed() < Duration::from_millis(120 * n as u64),
        "not serial"
    );
}

#[test]
fn unsupported_filters_fail_loudly_instead_of_dropping_them() {
    let c = corpus("filters", 61);
    let set = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    let f = Filters::new(
        vec![],
        vec!["nixos".into()],
        vec![],
        vec![],
        "personal",
        None,
        None,
    )
    .unwrap();
    let err = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &Rng(1).vec(),
            shard_top_k: 5,
            global_top_k: 5,
            filters: &f,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .err()
    .unwrap();
    assert!(
        err.to_string().contains("does not support classification"),
        "{err}"
    );
}

#[test]
fn failing_shard_fails_the_query() {
    let c = corpus("failing", 71);
    let set = c
        .discover(&LogConnector::new(vec![]), Tier::Internal)
        .unwrap();
    // Break one shard after discovery: drop its vec table.
    let con = rusqlite::Connection::open(c.dir.join("atlas-core-internal-work-02.db")).unwrap();
    con.execute_batch(&format!("DROP TABLE {DEFAULT_VEC_TABLE}"))
        .unwrap();
    let f = filters("personal");
    let err = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &Rng(2).vec(),
            shard_top_k: 5,
            global_top_k: 5,
            filters: &f,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .err()
    .unwrap();
    assert!(
        err.to_string().contains("atlas-core-internal-work-02"),
        "{err}"
    );
}

// ------------------------------------------------- ShardedBackend -------

/// Stands in for the monolith: serves FTS pools, records branch calls.
struct Monolith {
    seen: Mutex<Vec<(Option<String>, bool)>>,
}
impl SearchBackend for Monolith {
    fn name(&self) -> &'static str {
        "turso"
    }
    fn vec_table(&self) -> &str {
        DEFAULT_VEC_TABLE
    }
    fn notes(&self) -> Vec<String> {
        vec!["monolith note".into()]
    }
    fn branch_pools(&self, q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
        self.seen
            .lock()
            .unwrap()
            .push((q.match_expr.clone(), q.query_vector.is_some()));
        let mut p = BranchPools::default();
        if q.match_expr.is_some() {
            p.fts = vec![BranchHit {
                chunk_id: 999,
                dist: None,
            }];
        }
        if q.query_vector.is_some() {
            p.vector = vec![BranchHit {
                chunk_id: 888,
                dist: Some(0.0),
            }];
        }
        Ok(p)
    }
    fn chunk_details(&self, _ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(vec![])
    }
    fn neighbors(&self, _m: &str, _c: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        Ok(vec![])
    }
    fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
        Ok(None)
    }
}

#[test]
fn sharded_backend_takes_vector_from_shards_and_fts_from_the_monolith() {
    let c = corpus("backend", 81);
    let set = Arc::new(
        c.discover(&LogConnector::new(vec![Tier::Secret]), Tier::Internal)
            .unwrap(),
    );
    let be = ShardedBackend::new(
        Monolith {
            seen: Mutex::new(vec![]),
        },
        set,
    );
    let q = Rng(3).vec();
    let f = filters("personal");
    let pools = be
        .branch_pools(
            &BranchQuery {
                match_expr: Some("nix".into()),
                query_vector: Some(q.clone()),
                pool: 10,
                shard_top_k: 10,
                global_top_k: 10,
                vector_backend: VectorBackend::Scan,
            },
            &f,
        )
        .unwrap();
    assert_eq!(pools.fts[0].chunk_id, 999);
    let want = c.oracle_top(&f, &q, 10);
    let got: Vec<i64> = pools.vector.iter().map(|h| h.chunk_id).collect();
    assert_eq!(got, want.iter().map(|w| w.0).collect::<Vec<_>>());
    assert!(!got.contains(&888), "monolith vector pool is not consulted");
    // The monolith saw exactly one call: FTS only, no vector.
    assert_eq!(*be_inner_seen(&be), vec![(Some("nix".to_string()), false)]);
    assert!(
        be.notes()
            .iter()
            .any(|n| n.contains("shard fan-out over 5 shards"))
    );
    assert!(be.notes().contains(&"monolith note".to_string()));

    // Ceiling comes from the caller's filters: secret-bearing against an
    // internal-only consumer is refused.
    let err = be
        .branch_pools(
            &BranchQuery {
                match_expr: None,
                query_vector: Some(q.clone()),
                pool: 10,
                shard_top_k: 10,
                global_top_k: 10,
                vector_backend: VectorBackend::Scan,
            },
            &filters("secret-bearing"),
        )
        .err()
        .unwrap();
    assert!(matches!(err, Error::Fatal(_)), "{err}");

    // Index strategies are not offered on the shard path.
    let err = be
        .branch_pools(
            &BranchQuery {
                match_expr: None,
                query_vector: Some(q),
                pool: 10,
                shard_top_k: 10,
                global_top_k: 10,
                vector_backend: VectorBackend::IndexFill,
            },
            &f,
        )
        .err()
        .unwrap();
    assert!(err.to_string().contains("exact-cosine scan only"), "{err}");

    // FTS-only never touches the shards.
    let pools = be
        .branch_pools(
            &BranchQuery {
                match_expr: Some("x".into()),
                query_vector: None,
                pool: 5,
                shard_top_k: 10,
                global_top_k: 10,
                vector_backend: VectorBackend::Scan,
            },
            &f,
        )
        .unwrap();
    assert!(pools.vector.is_empty() && pools.fts.len() == 1);
}

fn be_inner_seen(
    be: &ShardedBackend<Monolith>,
) -> std::sync::MutexGuard<'_, Vec<(Option<String>, bool)>> {
    // The first (hybrid) call is the only one recorded at this point.
    be.inner().seen.lock().unwrap()
}

#[test]
fn static_lister_and_unknown_hosts_work_like_any_lister() {
    // A lister can name hosted shards without opening them: tiers with no
    // credential are filtered before any connect (nothing to dial here).
    let lister = StaticLister(vec![ShardEndpoint {
        name: "atlas-core-secret-main-01".into(),
        location: ShardLocation::Turso {
            host: "example.invalid".into(),
        },
    }]);
    let conn = LogConnector::new(vec![Tier::Secret]);
    let set = discover(
        &lister,
        &conn,
        &DiscoveryOptions::new(TierCeiling(Tier::Internal)),
    )
    .unwrap();
    assert!(set.shards().is_empty());
    assert!(conn.connected().is_empty());
}
