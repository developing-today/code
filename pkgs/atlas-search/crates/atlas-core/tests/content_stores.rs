//! Tier content-store hydration tests (developing-today/code#6664) —
//! no live Turso, no secrets: content stores are local SQLite files
//! in the monolith's published shape, and the statements are the
//! very ones sent to a hosted store.
//!
//! Oracles:
//! * hand-planted message/session/chunk rows, read back through the
//!   connector (hydration correctness);
//! * the connect log of a recording connector (physical ceiling: a
//!   content store above the ceiling is never connected);
//! * a marker-returning monolith stub (the ShardedBackend switch:
//!   enabled = content stores answer, disabled = monolith answers,
//!   byte-for-byte the pre-#6664 behaviour).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_core::backend::SearchBackend;
use atlas_core::filters::Filters;
use atlas_core::shards::{
    DiscoveryOptions, LocalShardTransport, ShardConnector, ShardEndpoint, ShardLocation,
    ShardSet, ShardTransport, ShardedBackend, StaticLister, Tier, TierCeiling,
    content_store_name, discover, discover_content_stores, parse_content_store_name,
};
use atlas_core::types::{
    BranchPools, BranchQuery, ChunkDetail, DEFAULT_VEC_TABLE, MessageRow, NeighborRow,
};
use atlas_core::{Error, Result};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-content-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// One planted conversation: a session, one message, its chunks.
struct Planted {
    session_id: &'static str,
    session_title: &'static str,
    message_id: &'static str,
    message_text: &'static str,
    chunks: Vec<(i64, i64, &'static str)>, // (chunk id, chunk_idx, text)
    sensitivity: &'static str,
}

fn write_content_store(dir: &Path, tier: Tier, p: &Planted) -> ShardEndpoint {
    let name = content_store_name(tier);
    let path = dir.join(format!("{name}.db"));
    let _ = std::fs::remove_file(&path);
    let con = rusqlite::Connection::open(&path).expect("create content store");
    con.execute_batch(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT, role TEXT, text TEXT);
         CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, client TEXT, created_at TEXT);
         CREATE TABLE session_labels (session_id TEXT PRIMARY KEY, kind TEXT, topics TEXT, \
           projects TEXT, sensitivity TEXT);",
    )
    .expect("schema");
    con.execute(
        "INSERT INTO sessions VALUES (?, ?, 'test-client', '2026-11-01T00:00:00Z')",
        rusqlite::params![p.session_id, p.session_title],
    )
    .expect("session");
    con.execute(
        "INSERT INTO messages VALUES (?, ?, 'user', ?)",
        rusqlite::params![p.message_id, p.session_id, p.message_text],
    )
    .expect("message");
    con.execute(
        "INSERT INTO session_labels VALUES (?, 'chat', '[]', '[]', ?)",
        rusqlite::params![p.session_id, p.sensitivity],
    )
    .expect("labels");
    for (id, idx, text) in &p.chunks {
        con.execute(
            "INSERT INTO chunks VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params![id, p.message_id, idx, text, format!("h{id}"), p.sensitivity],
        )
        .expect("chunk");
    }
    ShardEndpoint {
        name,
        location: ShardLocation::File(path),
    }
}

/// Connector with a connect log; `closed` tiers have no credential.
struct LogConnector {
    closed: Vec<Tier>,
    connects: Mutex<Vec<String>>,
}

impl LogConnector {
    fn new(closed: Vec<Tier>) -> LogConnector {
        LogConnector {
            closed,
            connects: Mutex::new(Vec::new()),
        }
    }
    fn connected(&self) -> Vec<String> {
        let mut v = self.connects.lock().unwrap().clone();
        v.sort();
        v
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
        Ok(Arc::new(LocalShardTransport::open(p)?))
    }
}

fn internal_plant() -> Planted {
    Planted {
        session_id: "s1",
        session_title: "Internal Session",
        message_id: "m1",
        message_text: "full internal message",
        chunks: vec![(7, 0, "chunk seven text"), (8, 1, "chunk eight text")],
        sensitivity: "personal",
    }
}

fn secret_plant() -> Planted {
    Planted {
        session_id: "s9",
        session_title: "Secret Session",
        message_id: "m9",
        message_text: "secret message text",
        chunks: vec![(99, 0, "secret chunk text")],
        sensitivity: "secret-bearing",
    }
}

#[test]
fn content_store_names_parse_and_round_trip() {
    assert_eq!(
        parse_content_store_name("atlas-core-internal-content"),
        Some(Tier::Internal)
    );
    assert_eq!(
        parse_content_store_name("atlas-core-secret-content"),
        Some(Tier::Secret)
    );
    // Scan shards and other names are not content stores.
    assert_eq!(parse_content_store_name("atlas-core-internal-main-01"), None);
    assert_eq!(parse_content_store_name("atlas-core-internal-content-01"), None);
    assert_eq!(parse_content_store_name("something-else"), None);
    assert_eq!(
        content_store_name(Tier::Public),
        "atlas-core-public-content"
    );
}

#[test]
fn hydration_reads_messages_and_sessions_from_the_admitted_tier_store() {
    let dir = temp_dir("hydrate");
    let ep = write_content_store(&dir, Tier::Internal, &internal_plant());
    let conn = LogConnector::new(vec![]);
    let stores = discover_content_stores(
        &StaticLister(vec![ep]),
        &conn,
        TierCeiling(Tier::Internal),
    )
    .unwrap();
    assert_eq!(stores.stores().len(), 1);
    assert!(stores.rejections.is_empty());

    let details = stores.chunk_details(&[7, 8]).unwrap();
    assert_eq!(details.len(), 2);
    let d7 = details.iter().find(|d| d.chunk_id == 7).unwrap();
    assert_eq!(d7.text, "chunk seven text");
    assert_eq!(d7.message_id, "m1");
    assert_eq!(d7.session_id, "s1");
    assert_eq!(d7.s_title.as_deref(), Some("Internal Session"));
    assert_eq!(d7.l_kind.as_deref(), Some("chat"));
    assert_eq!(d7.eff_sensitivity.as_deref(), Some("personal"));

    let msg = stores.message("m1").unwrap().expect("message m1");
    assert_eq!(msg.text, "full internal message");
    assert_eq!(msg.role.as_deref(), Some("user"));
    assert!(stores.message("absent").unwrap().is_none());

    let nb = stores.neighbors("m1", 0, 1).unwrap();
    assert_eq!(
        nb.iter().map(|r| r.chunk_id).collect::<Vec<_>>(),
        [7, 8]
    );
}

#[test]
fn above_ceiling_content_store_is_never_connected() {
    let dir = temp_dir("ceiling");
    let eps = vec![
        write_content_store(&dir, Tier::Internal, &internal_plant()),
        write_content_store(&dir, Tier::Secret, &secret_plant()),
    ];
    // The connector COULD open secret (it has every credential);
    // the ceiling alone must keep the secret store unconnected.
    let conn = LogConnector::new(vec![]);
    let stores = discover_content_stores(
        &StaticLister(eps),
        &conn,
        TierCeiling(Tier::Internal),
    )
    .unwrap();
    assert_eq!(conn.connected(), ["atlas-core-internal-content"]);
    assert_eq!(stores.stores().len(), 1);
    assert!(
        stores
            .rejections
            .iter()
            .any(|r| r.name == "atlas-core-secret-content"
                && r.reason.contains("above the ceiling")),
        "{:?}",
        stores.rejections
    );
    // And nothing secret can be hydrated through the admitted set.
    assert!(stores.chunk_details(&[99]).unwrap().is_empty());
    assert!(stores.message("m9").unwrap().is_none());
    assert!(stores.neighbors("m9", 0, 3).unwrap().is_empty());
}

#[test]
fn tier_without_credential_is_rejected_without_connecting() {
    let dir = temp_dir("nocred");
    let eps = vec![
        write_content_store(&dir, Tier::Public, &internal_plant()),
        write_content_store(&dir, Tier::Internal, &internal_plant()),
    ];
    let conn = LogConnector::new(vec![Tier::Public]);
    let stores = discover_content_stores(
        &StaticLister(eps),
        &conn,
        TierCeiling(Tier::Internal),
    )
    .unwrap();
    assert_eq!(conn.connected(), ["atlas-core-internal-content"]);
    assert!(
        stores
            .rejections
            .iter()
            .any(|r| r.name == "atlas-core-public-content"
                && r.reason.contains("no public-tier credential")),
        "{:?}",
        stores.rejections
    );
}

/// A store that connects but has no `messages` table is rejected at
/// discovery with the reason recorded; a store missing `chunks`
/// fails hydration with a named error. Degradation is explicit,
/// never silent, and never served from another tier's store.
#[test]
fn broken_content_stores_degrade_explicitly() {
    let dir = temp_dir("broken");
    let bad_path = dir.join("atlas-core-public-content.db");
    {
        let con = rusqlite::Connection::open(&bad_path).expect("create");
        con.execute_batch("CREATE TABLE unrelated (x INTEGER);")
            .expect("schema");
    }
    let bad_ep = ShardEndpoint {
        name: "atlas-core-public-content".to_string(),
        location: ShardLocation::File(bad_path),
    };
    let conn = LogConnector::new(vec![]);
    let stores = discover_content_stores(
        &StaticLister(vec![bad_ep]),
        &conn,
        TierCeiling(Tier::Internal),
    )
    .unwrap();
    assert!(stores.stores().is_empty());
    assert!(
        stores
            .rejections
            .iter()
            .any(|r| r.name == "atlas-core-public-content"
                && r.reason.contains("unreadable")),
        "{:?}",
        stores.rejections
    );
    // Empty admitted set: hydration is empty, not an error and
    // never filled from anywhere else.
    assert!(stores.chunk_details(&[7]).unwrap().is_empty());

    // Messages present (probe passes), chunks missing: the failure
    // names the store.
    let half_path = dir.join("atlas-core-internal-content.db");
    {
        let con = rusqlite::Connection::open(&half_path).expect("create");
        con.execute_batch(
            "CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT, role TEXT, text TEXT);",
        )
        .expect("schema");
    }
    let half_ep = ShardEndpoint {
        name: "atlas-core-internal-content".to_string(),
        location: ShardLocation::File(half_path),
    };
    let stores = discover_content_stores(
        &StaticLister(vec![half_ep]),
        &conn,
        TierCeiling(Tier::Internal),
    )
    .unwrap();
    assert_eq!(stores.stores().len(), 1);
    let err = stores.chunk_details(&[7]).unwrap_err();
    assert!(
        err.to_string().contains("atlas-core-internal-content"),
        "{err}"
    );
}

/// The wrapped-monolith stub: every hydration answer is a marker,
/// so a test can tell exactly which side served a call.
struct Monolith;

impl SearchBackend for Monolith {
    fn name(&self) -> &'static str {
        "monolith-stub"
    }
    fn vec_table(&self) -> &str {
        DEFAULT_VEC_TABLE
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(&self, _q: &BranchQuery, _f: &Filters) -> Result<BranchPools> {
        Ok(BranchPools::default())
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(ids
            .iter()
            .map(|id| ChunkDetail {
                chunk_id: *id,
                text: "MONOLITH".to_string(),
                chunk_idx: 0,
                message_id: "monolith".to_string(),
                m_role: None,
                session_id: "monolith".to_string(),
                s_title: None,
                s_client: None,
                s_created_at: None,
                l_kind: None,
                l_topics: None,
                l_projects: None,
                eff_sensitivity: None,
            })
            .collect())
    }
    fn neighbors(&self, _m: &str, _c: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        Ok(vec![NeighborRow {
            chunk_id: -1,
            chunk_idx: 0,
            text: "MONOLITH".to_string(),
        }])
    }
    fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
        Ok(Some(MessageRow {
            id: "monolith".to_string(),
            role: None,
            text: "MONOLITH".to_string(),
        }))
    }
}

/// One real scan shard so a ShardSet can be discovered, mirroring
/// the tests/shards.rs fixture shape.
fn write_scan_shard(dir: &Path) -> ShardEndpoint {
    let name = "atlas-core-internal-main-01";
    let path = dir.join(format!("{name}.db"));
    let _ = std::fs::remove_file(&path);
    let con = rusqlite::Connection::open(&path).expect("create shard");
    con.execute_batch(&format!(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
         CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);"
    ))
    .expect("schema");
    for (k, v) in [
        ("shard", name),
        ("tier", "internal"),
        ("topic", "main"),
        ("part", "1"),
        ("expected_chunks", "0"),
        ("metric", "cosine"),
        ("embedding_model", "snowflake-arctic-embed-m-v2.0"),
        ("engine", "tursodb"),
    ] {
        con.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES (?, ?)",
            rusqlite::params![k, v],
        )
        .expect("manifest");
    }
    ShardEndpoint {
        name: name.to_string(),
        location: ShardLocation::File(path),
    }
}

fn discovered_set(dir: &Path) -> Arc<ShardSet> {
    let conn = LogConnector::new(vec![]);
    let set = discover(
        &StaticLister(vec![write_scan_shard(dir)]),
        &conn,
        &DiscoveryOptions::new(TierCeiling(Tier::Internal)),
    )
    .unwrap();
    Arc::new(set)
}

#[test]
fn sharded_backend_hydrates_from_content_stores_only_when_enabled() {
    let dir = temp_dir("switch");
    let set = discovered_set(&dir);
    let conn = LogConnector::new(vec![]);
    let stores = Arc::new(
        discover_content_stores(
            &StaticLister(vec![write_content_store(
                &dir,
                Tier::Internal,
                &internal_plant(),
            )]),
            &conn,
            TierCeiling(Tier::Internal),
        )
        .unwrap(),
    );

    // Enabled: hydration comes from the content store.
    let be = ShardedBackend::new(Monolith, set.clone())
        .with_content_stores(stores.clone(), true);
    let details = be.chunk_details(&[7]).unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].text, "chunk seven text");
    assert_eq!(be.message("m1").unwrap().unwrap().text, "full internal message");
    assert_eq!(
        be.neighbors("m1", 0, 1)
            .unwrap()
            .iter()
            .map(|r| r.chunk_id)
            .collect::<Vec<_>>(),
        [7, 8]
    );
    assert!(
        be.notes()
            .iter()
            .any(|n| n.contains("hydration served by tier content stores")),
        "{:?}",
        be.notes()
    );

    // Disabled (the shipped default): the monolith answers, exactly
    // as before #6664.
    let be = ShardedBackend::new(Monolith, set).with_content_stores(stores, false);
    let details = be.chunk_details(&[7]).unwrap();
    assert_eq!(details[0].text, "MONOLITH");
    assert_eq!(be.message("m1").unwrap().unwrap().text, "MONOLITH");
    assert_eq!(be.neighbors("m1", 0, 1).unwrap()[0].text, "MONOLITH");

    // The Error type is linked for unwrap_err users above.
    let _ = Error::fatal("unused");
}
