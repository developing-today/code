//! Committed-catalog visibility gate tests (design #6057 §6.3, read
//! side; issue #12406) — no live Turso, no secrets: shards are local
//! SQLite files and the catalog is built by hand.
//!
//! Oracles (the testing doctrine asks each test to name one):
//! * the connect log of a recording connector (fail-closed *before*
//!   probing: an uncommitted physical shard is never connected);
//! * hand-computed shard-name sets and rejection reasons;
//! * catalog snapshot determinism across entry order and across
//!   uncommitted on-disk additions (committed id moves only with the
//!   generation).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use atlas_core::shards::{
    CommittedCatalog, CommittedShard, DirLister, DiscoveryOptions, EMBEDDING_MODEL_ID,
    LocalShardTransport, ShardConnector, ShardEndpoint, ShardLocation, ShardSet, Tier, TierCeiling,
    discover, parse_shard_name,
};
use atlas_core::types::DEFAULT_VEC_TABLE;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-catalog-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn write_shard(dir: &Path, name: &str, n_chunks: usize) {
    let (tier, topic, part) = parse_shard_name(name).expect("shard name");
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
    for i in 0..n_chunks {
        con.execute(
            "INSERT INTO chunks VALUES (?, ?, 0, ?, ?, ?)",
            rusqlite::params![
                i as i64 + 1,
                format!("m{i}"),
                "t",
                format!("h{i}"),
                "personal"
            ],
        )
        .expect("chunk");
    }
    for (k, v) in [
        ("shard", name.to_string()),
        ("tier", tier.name().to_string()),
        ("topic", topic),
        ("part", part.to_string()),
        ("expected_chunks", n_chunks.to_string()),
        ("metric", "cosine".to_string()),
        ("embedding_model", EMBEDDING_MODEL_ID.to_string()),
    ] {
        con.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES (?, ?)",
            rusqlite::params![k, v],
        )
        .expect("manifest");
    }
}

/// Connector that opens every tier and logs every connection.
struct LogConnector {
    connects: Mutex<Vec<String>>,
}

impl LogConnector {
    fn new() -> LogConnector {
        LogConnector {
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
    fn can_open(&self, _tier: Tier) -> bool {
        true
    }
    fn connect(
        &self,
        endpoint: &ShardEndpoint,
        _tier: Tier,
    ) -> Result<Arc<dyn atlas_core::shards::ShardTransport>, atlas_core::Error> {
        self.connects.lock().unwrap().push(endpoint.name.clone());
        let ShardLocation::File(p) = &endpoint.location else {
            panic!("tests use files");
        };
        Ok(Arc::new(LocalShardTransport::open(p)?))
    }
}

fn committed_entry(name: &str, n_chunks: u64) -> CommittedShard {
    let (tier, topic, part) = parse_shard_name(name).expect("shard name");
    CommittedShard {
        name: name.to_string(),
        tier,
        topic,
        part,
        expected_chunks: Some(n_chunks),
        embedding_model: Some(EMBEDDING_MODEL_ID.to_string()),
    }
}

fn catalog(generation: u64, names: &[(&str, u64)]) -> CommittedCatalog {
    CommittedCatalog {
        format: atlas_core::shards::CATALOG_FORMAT.to_string(),
        generation,
        shards: names.iter().map(|(n, c)| committed_entry(n, *c)).collect(),
    }
}

fn discover_pinned(
    dir: &Path,
    conn: &LogConnector,
    cat: &CommittedCatalog,
    lenient: bool,
) -> Result<ShardSet, atlas_core::Error> {
    let mut opts = DiscoveryOptions::new(TierCeiling(Tier::Secret));
    opts.catalog = Some(cat.clone());
    opts.lenient = lenient;
    discover(&DirLister { dir: dir.into() }, conn, &opts)
}

fn names(set: &ShardSet) -> Vec<&str> {
    set.shards().iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn pinned_discovery_serves_only_committed_shards() {
    let dir = temp_dir("pinned");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    write_shard(&dir, "atlas-core-internal-media-01", 2);
    write_shard(&dir, "atlas-core-internal-misc-01", 1);
    let cat = catalog(
        7,
        &[
            ("atlas-core-internal-work-01", 4),
            ("atlas-core-internal-media-01", 2),
        ],
    );
    let conn = LogConnector::new();
    let set = discover_pinned(&dir, &conn, &cat, true).expect("pinned discovery");

    // Oracle: hand-computed names; the connect log proves the
    // uncommitted shard was refused before any probe or scan.
    assert_eq!(
        names(&set),
        [
            "atlas-core-internal-media-01",
            "atlas-core-internal-work-01"
        ]
    );
    assert!(
        set.rejections
            .iter()
            .any(|r| r.name == "atlas-core-internal-misc-01"
                && r.reason.contains("committed catalog generation 7")),
        "uncommitted shard is an explicit rejection: {:?}",
        set.rejections
    );
    assert!(
        !conn
            .connected()
            .contains(&"atlas-core-internal-misc-01".to_string()),
        "uncommitted shard is never connected: {:?}",
        conn.connected()
    );
    assert_eq!(set.catalog_generation(), Some(7));
    assert_eq!(set.catalog_snapshot_id(), Some(cat.snapshot_id()));
}

#[test]
fn uncommitted_shard_is_fatal_without_lenient() {
    let dir = temp_dir("strict");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    write_shard(&dir, "atlas-core-internal-misc-01", 1);
    let cat = catalog(3, &[("atlas-core-internal-work-01", 4)]);
    let conn = LogConnector::new();
    let err = match discover_pinned(&dir, &conn, &cat, false) {
        Ok(_) => panic!("strict must fail closed"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("atlas-core-internal-misc-01")
            && err.to_string().contains("not in committed catalog"),
        "strict error names the uncommitted shard: {err}"
    );
}

#[test]
fn missing_committed_shard_is_reported_never_silent() {
    let dir = temp_dir("missing");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    let cat = catalog(
        5,
        &[
            ("atlas-core-internal-work-01", 4),
            ("atlas-core-internal-media-01", 2),
        ],
    );
    // Lenient: explicit rejection, served set holds only what exists.
    let conn = LogConnector::new();
    let set = discover_pinned(&dir, &conn, &cat, true).expect("lenient discovery");
    assert_eq!(names(&set), ["atlas-core-internal-work-01"]);
    assert!(
        set.rejections
            .iter()
            .any(|r| r.name == "atlas-core-internal-media-01" && r.reason.contains("not listed")),
        "missing committed shard is an explicit rejection: {:?}",
        set.rejections
    );
    // Strict: a fan-out over fewer shards than committed is refused.
    let err = match discover_pinned(&dir, &LogConnector::new(), &cat, false) {
        Ok(_) => panic!("strict must fail closed"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("atlas-core-internal-media-01"),
        "strict error names the missing shard: {err}"
    );
}

#[test]
fn manifest_catalog_mismatch_is_refused() {
    let dir = temp_dir("mismatch");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    // Catalog pins 9 chunks; the manifest (and disk) say 4: prepare
    // and commit disagree, so the shard must be refused.
    let cat = catalog(9, &[("atlas-core-internal-work-01", 9)]);
    let conn = LogConnector::new();
    let set = discover_pinned(&dir, &conn, &cat, true).expect("lenient discovery");
    // Oracle: the shard WAS connected (the manifest was read and
    // cross-checked) but is not served.
    assert!(names(&set).is_empty());
    assert!(
        set.rejections.iter().any(
            |r| r.name == "atlas-core-internal-work-01" && r.reason.contains("expected_chunks")
        ),
        "mismatch is an explicit rejection: {:?}",
        set.rejections
    );
    assert!(
        conn.connected()
            .contains(&"atlas-core-internal-work-01".to_string()),
        "refusal happens after the manifest cross-check: {:?}",
        conn.connected()
    );
    let err = match discover_pinned(&dir, &LogConnector::new(), &cat, false) {
        Ok(_) => panic!("strict must fail closed"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("expected_chunks"), "{err}");
}

#[test]
fn catalog_snapshot_comes_from_the_generation_not_the_disk() {
    let dir = temp_dir("snapshot");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    let cat = catalog(11, &[("atlas-core-internal-work-01", 4)]);
    let conn = LogConnector::new();
    let set = discover_pinned(&dir, &conn, &cat, true).expect("pinned discovery");

    // Oracle: determinism + independence. The committed id is stable
    // across repeat discoveries and across entry order, and an
    // uncommitted on-disk addition moves the physical snapshot but
    // not the committed one.
    let again = discover_pinned(&dir, &LogConnector::new(), &cat, true).expect("repeat");
    assert_eq!(set.catalog_snapshot_id(), again.catalog_snapshot_id());
    let mut reordered = cat.clone();
    reordered.shards.reverse();
    assert_eq!(cat.snapshot_id(), reordered.snapshot_id(), "order-free");
    let physical_before = set.snapshot_id();

    write_shard(&dir, "atlas-core-internal-misc-01", 1);
    let after = discover_pinned(&dir, &LogConnector::new(), &cat, true).expect("after addition");
    assert_eq!(
        after.catalog_snapshot_id(),
        set.catalog_snapshot_id(),
        "uncommitted addition does not move the committed id"
    );
    assert_eq!(
        after.snapshot_id(),
        physical_before,
        "a pinned set observes the commit, not the disk: the refused shard is in neither snapshot"
    );
    // The ungated path still observes the disk (legacy behavior).
    let ungated = discover(
        &DirLister { dir: dir.into() },
        &LogConnector::new(),
        &DiscoveryOptions::new(TierCeiling(Tier::Secret)),
    )
    .expect("ungated");
    assert_ne!(
        ungated.snapshot_id(),
        physical_before,
        "ungated discovery still observes the disk"
    );
    assert!(
        after
            .rejections
            .iter()
            .any(|r| r.name == "atlas-core-internal-misc-01"),
        "the addition is still refused: {:?}",
        after.rejections
    );
}

#[test]
fn unpinned_discovery_is_unchanged() {
    // Regression guard: without a catalog, physical enumeration
    // serves every well-formed shard exactly as before.
    let dir = temp_dir("ungated");
    write_shard(&dir, "atlas-core-internal-work-01", 4);
    write_shard(&dir, "atlas-core-internal-media-01", 2);
    let conn = LogConnector::new();
    let opts = DiscoveryOptions::new(TierCeiling(Tier::Secret));
    let set = discover(&DirLister { dir: dir.into() }, &conn, &opts).expect("ungated");
    assert_eq!(
        names(&set),
        [
            "atlas-core-internal-media-01",
            "atlas-core-internal-work-01"
        ]
    );
    assert_eq!(set.catalog_generation(), None);
    assert_eq!(set.catalog_snapshot_id(), None);
}
