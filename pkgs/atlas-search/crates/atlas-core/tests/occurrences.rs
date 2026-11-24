//! Oracle tests for reading the `chunk_occurrences` provenance
//! registry (#3125 slice 3): the registry slice 2 writes becomes
//! readable through the shard set, tolerant of shards that
//! predate the registry, and surfaces in `get-chunk` output.
//!
//! Oracle: content identity by SHA-256 — the canonical hash the
//! aggregator records is the SHA-256 of the piece text, so a
//! lookup by that hash must find every shard row that recorded
//! it, regardless of which shard or item id the piece landed on.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use atlas_core::error::Result;
use atlas_core::filters::Sensitivity;
use atlas_core::occurrences::{ChunkOccurrence, lookup_by_hash};
use atlas_core::search::run_get_chunk;
use atlas_core::shards::scan::ShardedBackend;
use atlas_core::shards::{
    DirLister, DiscoveryOptions, ShardConnector, ShardEndpoint, ShardSet, Tier, TierCeiling,
    discover,
};
use atlas_core::types::{
    BranchHit, BranchPools, BranchQuery, ChunkDetail, MessageRow, NeighborRow,
};
use atlas_core::{DEFAULT_VEC_TABLE, SearchBackend};

const NOW: &str = "2026-11-23T00:00:00Z";
const TEXT: &str = "dns shared block\n";
/// SHA-256 of TEXT — the canonical content hash the aggregator
/// records (the oracle: content identity).
const HASH: &str = "ecead345a7a4b99b26d42d2e839838ee261516fe54136d287847852a0ffe4f9f";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-occ-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// One (source, span) pair a fixture shard records for HASH.
fn fixture(dir: &Path, name: &str, occurrences: &[(&str, i64, i64)], legacy: bool) -> PathBuf {
    fixture_with_sens(dir, name, occurrences, legacy, &["personal"])
}

/// Same as [`fixture`], but each chunk row's `sensitivity` is taken
/// from `sens` positionally (cycling when shorter): the per-row
/// classification promotion writes separately from the shard tier
/// (#3125 slice 3 P1: an admitted shard can hold rows above the
/// caller's ceiling).
fn fixture_with_sens(
    dir: &Path,
    name: &str,
    occurrences: &[(&str, i64, i64)],
    legacy: bool,
    sens: &[&str],
) -> PathBuf {
    use rusqlite::params;
    let (tier, topic, part) =
        atlas_core::shards::parse_shard_name(name).expect("shard name parses");
    let path = dir.join(format!("{name}.db"));
    let _ = std::fs::remove_file(&path);
    let con = rusqlite::Connection::open(&path).expect("create store");
    con.execute_batch(&format!(
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, message_id TEXT NOT NULL, \
           chunk_idx INTEGER, text TEXT NOT NULL, content_hash TEXT UNIQUE NOT NULL, \
           sensitivity TEXT);
         CREATE TABLE {DEFAULT_VEC_TABLE} (chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL);
         CREATE TABLE atlas_manifest (\"key\" TEXT PRIMARY KEY, value TEXT);"
    ))
    .expect("schema");
    if !legacy {
        con.execute_batch(
            "CREATE TABLE chunk_occurrences ( \
               shard_chunk_id INTEGER NOT NULL REFERENCES chunks(id) ON DELETE CASCADE, \
               canonical_hash TEXT NOT NULL, source TEXT NOT NULL, \
               span_start INTEGER NOT NULL, span_end INTEGER NOT NULL, observed_at TEXT NOT NULL, \
               PRIMARY KEY (shard_chunk_id, source, span_start)); \
             CREATE INDEX chunk_occurrences_hash ON chunk_occurrences (canonical_hash);",
        )
        .expect("occurrence schema");
    }
    for (i, (source, start, end)) in occurrences.iter().enumerate() {
        let id = (i + 1) as i64;
        let chunk_sens = sens.get(i % sens.len()).copied().unwrap_or("personal");
        con.execute(
            "INSERT INTO chunks VALUES (?1, ?2, 0, ?3, ?4, ?5)",
            params![id, source, TEXT, format!("h{id}"), chunk_sens],
        )
        .expect("chunk");
        con.execute(
            &format!("INSERT INTO {DEFAULT_VEC_TABLE} VALUES (?1, x'00000000')"),
            params![id],
        )
        .expect("vec");
        if !legacy {
            con.execute(
                "INSERT INTO chunk_occurrences \
                 (shard_chunk_id, canonical_hash, source, span_start, span_end, observed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, HASH, source, start, end, NOW],
            )
            .expect("occurrence");
        }
    }
    for (k, v) in [
        ("shard", name),
        ("tier", tier.name()),
        ("topic", &topic),
        ("part", &part.to_string()),
        ("expected_chunks", &occurrences.len().to_string()),
        ("metric", "cosine"),
        ("embedding_model", atlas_core::shards::EMBEDDING_MODEL_ID),
        ("engine", "tursodb"),
    ] {
        con.execute(
            "INSERT OR REPLACE INTO atlas_manifest VALUES (?, ?)",
            params![k, v],
        )
        .expect("manifest");
    }
    path
}

struct OpenConnector;
impl ShardConnector for OpenConnector {
    fn can_open(&self, _tier: Tier) -> bool {
        true
    }
    fn connect(
        &self,
        ep: &ShardEndpoint,
        _tier: Tier,
    ) -> Result<std::sync::Arc<dyn atlas_core::shards::ShardTransport>> {
        use atlas_core::shards::ShardLocation;
        let ShardLocation::File(p) = &ep.location else {
            panic!("tests use files");
        };
        Ok(std::sync::Arc::new(
            atlas_core::shards::LocalShardTransport::open(p)?,
        ))
    }
}

fn discover_set(dir: &Path, ceiling: Tier) -> ShardSet {
    discover(
        &DirLister {
            dir: dir.to_path_buf(),
        },
        &OpenConnector,
        &DiscoveryOptions::new(TierCeiling(ceiling)),
    )
    .expect("discovery")
}

#[test]
fn lookup_finds_every_occurrence_across_shards_and_skips_legacy_shards() {
    // #3125 rule 3: every chunk records EVERY place it was found.
    // The same text hash occurs in three registry-carrying shards
    // (internal x2, secret x1) plus one legacy shard with no
    // chunk_occurrences table at all — a lookup under an internal
    // ceiling returns exactly the two internal rows, and the
    // legacy shard is a note, not a failure.
    let dir = temp_dir("lookup");
    fixture(
        &dir,
        "atlas-core-internal-work-01",
        &[("repo@abc:infra/dns/a.yaml", 0, 17)],
        false,
    );
    fixture(
        &dir,
        "atlas-core-internal-media-01",
        &[("repo@abc:infra/dns/b.yaml", 0, 17)],
        false,
    );
    fixture(
        &dir,
        "atlas-core-secret-main-01",
        &[("repo@abc:infra/dns/secret.yaml", 0, 17)],
        false,
    );
    // Legacy: promoted before slice 2 — no registry table.
    fixture(&dir, "atlas-core-internal-misc-01", &[], true);
    let set = discover_set(&dir, Tier::Secret);

    let found = lookup_by_hash(
        &set,
        TierCeiling(Tier::Internal),
        Sensitivity::LocalOnly,
        HASH,
        None,
    )
    .unwrap();
    assert_eq!(
        found,
        vec![
            ChunkOccurrence {
                shard: "atlas-core-internal-media-01".into(),
                tier: Tier::Internal,
                canonical_hash: HASH.into(),
                source: "repo@abc:infra/dns/b.yaml".into(),
                span_start: 0,
                span_end: 17,
                observed_at: NOW.into(),
                root_id: None,
            },
            ChunkOccurrence {
                shard: "atlas-core-internal-work-01".into(),
                tier: Tier::Internal,
                canonical_hash: HASH.into(),
                source: "repo@abc:infra/dns/a.yaml".into(),
                span_start: 0,
                span_end: 17,
                observed_at: NOW.into(),
                root_id: None,
            },
        ],
        "internal rows only, ordered by shard; legacy shard contributes nothing and no error"
    );

    // A secret ceiling sees the third occurrence too.
    let secret = lookup_by_hash(
        &set,
        TierCeiling(Tier::Secret),
        Sensitivity::SecretBearing,
        HASH,
        None,
    )
    .unwrap();
    assert_eq!(secret.len(), 3, "secret ceiling adds the secret-tier row");

    // An unknown hash reads as empty, never as an error.
    assert!(
        lookup_by_hash(
            &set,
            TierCeiling(Tier::Internal),
            Sensitivity::LocalOnly,
            "00",
            None,
        )
        .unwrap()
        .is_empty()
    );
}

/// Stands in for the wrapped monolith: serves one chunk detail
/// whose text is the fixture text, records nothing else.
struct Monolith {
    chunk_id: i64,
}
impl SearchBackend for Monolith {
    fn name(&self) -> &'static str {
        "turso"
    }
    fn vec_table(&self) -> &str {
        DEFAULT_VEC_TABLE
    }
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
    fn branch_pools(
        &self,
        _q: &BranchQuery,
        _f: &atlas_core::filters::Filters,
    ) -> Result<BranchPools> {
        Ok(BranchPools::default())
    }
    fn chunk_details(&self, ids: &[i64]) -> Result<Vec<ChunkDetail>> {
        Ok(ids
            .iter()
            .filter(|id| **id == self.chunk_id)
            .map(|id| ChunkDetail {
                chunk_id: *id,
                text: TEXT.into(),
                chunk_idx: 0,
                message_id: "m1".into(),
                m_role: Some("user".into()),
                session_id: "s1".into(),
                s_title: None,
                s_client: None,
                s_created_at: None,
                l_kind: None,
                l_topics: None,
                l_projects: None,
                eff_sensitivity: Some("personal".into()),
            })
            .collect())
    }
    fn neighbors(&self, _m: &str, _c: i64, _n: i64) -> Result<Vec<NeighborRow>> {
        Ok(Vec::new())
    }
    fn message(&self, _m: &str) -> Result<Option<MessageRow>> {
        Ok(None)
    }
}

#[test]
fn get_chunk_reports_occurrences_from_the_shard_fan_out() {
    // After slice 3, `get-chunk` resolves the chunk's canonical
    // hash through the serving SearchBackend and reports every
    // recorded place the text was found. On the wrapped monolith
    // alone (no shards, default trait method) the field is
    // absent; through the ShardedBackend it carries both internal
    // sources with their spans.
    let dir = temp_dir("getchunk");
    fixture(
        &dir,
        "atlas-core-internal-work-01",
        &[("repo@abc:infra/dns/a.yaml", 0, 17)],
        false,
    );
    fixture(
        &dir,
        "atlas-core-internal-media-01",
        &[("repo@abc:infra/dns/b.yaml", 4, 21)],
        false,
    );
    // The get-chunk path runs under the LocalOwner disclosure
    // context (ceiling secret-bearing), so the set must be
    // discovered with a secret-capable connector; fail-closed is
    // the correct answer otherwise.
    let set = std::sync::Arc::new(discover_set(&dir, Tier::Secret));

    let inner = Monolith { chunk_id: 7 };
    let out = run_get_chunk(&inner, 7, 0).unwrap();
    assert!(
        out.get("occurrences").is_none(),
        "no shards behind the monolith: the field is absent, not empty"
    );

    let sharded = ShardedBackend::new(inner, set);
    let out = run_get_chunk(&sharded, 7, 0).unwrap();
    let occ = out
        .get("occurrences")
        .expect("occurrences present")
        .as_array()
        .unwrap();
    assert_eq!(occ.len(), 2, "both internal shard rows surface");
    let sources: Vec<&str> = occ
        .iter()
        .filter_map(|o| o.get("source").and_then(|s| s.as_str()))
        .collect();
    assert_eq!(
        sources,
        ["repo@abc:infra/dns/b.yaml", "repo@abc:infra/dns/a.yaml"],
        "ordered by shard name (media < work)"
    );
    assert_eq!(
        occ[0].get("span_start").and_then(|v| v.as_i64()),
        Some(4),
        "the media shard's row carries its own span"
    );
    assert_eq!(occ[0].get("span_end").and_then(|v| v.as_i64()), Some(21));
    assert_eq!(
        occ[0].get("observed_at").and_then(|v| v.as_str()),
        Some(NOW)
    );
    assert_eq!(
        occ[0].get("shard").and_then(|v| v.as_str()),
        Some("atlas-core-internal-media-01")
    );
}

#[test]
fn sharded_backend_occurrences_respect_the_caller_ceiling() {
    // The SearchBackend seam routes occurrence reads through the
    // shard fan-out with the caller's ceiling: a secret-tier row
    // is invisible to an internal caller and visible to a
    // secret-bearing caller.
    let dir = temp_dir("ceiling");
    fixture(
        &dir,
        "atlas-core-internal-work-01",
        &[("repo@abc:infra/dns/a.yaml", 0, 17)],
        false,
    );
    fixture(
        &dir,
        "atlas-core-secret-main-01",
        &[("repo@abc:infra/dns/secret.yaml", 0, 17)],
        false,
    );
    let set = std::sync::Arc::new(discover_set(&dir, Tier::Secret));
    let sharded = ShardedBackend::new(Monolith { chunk_id: 1 }, set);

    let internal = sharded
        .occurrences(HASH, TierCeiling(Tier::Internal), Sensitivity::LocalOnly)
        .unwrap();
    assert_eq!(internal.len(), 1, "secret-tier occurrence withheld");
    let secret = sharded
        .occurrences(HASH, TierCeiling(Tier::Secret), Sensitivity::SecretBearing)
        .unwrap();
    assert_eq!(secret.len(), 2, "secret ceiling adds the secret row");
}

#[test]
fn occurrence_rows_are_filtered_by_their_own_chunk_sensitivity_not_just_shard_tier() {
    // #3125 slice 3 P1: promotion names a shard from the unit's
    // tier but writes each chunk's effective sensitivity
    // separately, so an admitted shard can hold rows above the
    // caller's ceiling. Both occurrences live in the SAME
    // internal shard (admitted by an internal tier ceiling); only
    // the per-row sensitivity decides which surface: a personal
    // caller must not see the local-only row, and the
    // personal/local-only distinction must survive (an internal
    // TierCeiling alone cannot tell them apart).
    let dir = temp_dir("rowsens");
    fixture_with_sens(
        &dir,
        "atlas-core-internal-work-01",
        &[
            ("repo@abc:infra/dns/shared.yaml", 0, 17),
            ("repo@abc:infra/dns/local.yaml", 0, 17),
        ],
        false,
        &["personal", "local-only"],
    );
    let set = discover_set(&dir, Tier::Internal);

    let personal = lookup_by_hash(
        &set,
        TierCeiling(Tier::Internal),
        Sensitivity::Personal,
        HASH,
        None,
    )
    .unwrap();
    assert_eq!(
        personal
            .iter()
            .map(|o| o.source.as_str())
            .collect::<Vec<_>>(),
        ["repo@abc:infra/dns/shared.yaml"],
        "personal caller sees only the personal row; the local-only row in the same admitted shard is withheld"
    );

    let local = lookup_by_hash(
        &set,
        TierCeiling(Tier::Internal),
        Sensitivity::LocalOnly,
        HASH,
        None,
    )
    .unwrap();
    assert_eq!(
        local.len(),
        2,
        "local-only ceiling sees both rows in the admitted shard"
    );
}

// Silence unused warnings for helpers the monolith mock keeps.
#[allow(dead_code)]
fn _unused(_r: &Mutex<()>) -> BranchHit {
    BranchHit {
        chunk_id: 0,
        dist: None,
    }
}

/// An in-memory source registry holding one root (`repo`, based at
/// `repo@abc:`) with one registered locator; any other locator is
/// unregistered by construction.
fn registry_with_one_root() -> rusqlite::Connection {
    use atlas_core::source_registry::{register_locator, register_source_root};
    let conn = rusqlite::Connection::open_in_memory().expect("in-memory registry");
    register_source_root(&conn, "repo", "repo@abc:", NOW).expect("root");
    register_locator(&conn, "repo", "repo@abc:infra/dns/a.yaml", NOW).expect("locator");
    conn
}

#[test]
fn lookup_attributes_occurrences_to_registered_roots_only() {
    // #3125 slice 6: with a registry attached, the occurrence whose
    // source locator is registered resolves to its root; the
    // unregistered locator stays unattributed (None), never guessed.
    // Oracle: slice 5's exact-match resolution contract — the
    // registry answers "which root holds this locator", and the
    // lookup carries that answer onto the row.
    let dir = temp_dir("attribution");
    fixture(
        &dir,
        "atlas-core-internal-work-01",
        &[
            ("repo@abc:infra/dns/a.yaml", 0, 17),
            ("repo@abc:infra/dns/b.yaml", 0, 17),
        ],
        false,
    );
    let set = discover_set(&dir, Tier::Internal);
    let registry = registry_with_one_root();

    let found = lookup_by_hash(
        &set,
        TierCeiling(Tier::Internal),
        Sensitivity::LocalOnly,
        HASH,
        Some(&registry),
    )
    .unwrap();
    assert_eq!(found.len(), 2, "both shard rows surface under the ceiling");
    let by_source: std::collections::HashMap<&str, &ChunkOccurrence> =
        found.iter().map(|o| (o.source.as_str(), o)).collect();
    assert_eq!(
        by_source["repo@abc:infra/dns/a.yaml"].root_id.as_deref(),
        Some("repo"),
        "registered locator attributes to its root"
    );
    assert_eq!(
        by_source["repo@abc:infra/dns/b.yaml"].root_id, None,
        "unregistered locator stays unattributed, never misattributed"
    );

    // Without a registry the same lookup is unattributed and the
    // wire form carries no root_id key at all.
    let plain = lookup_by_hash(
        &set,
        TierCeiling(Tier::Internal),
        Sensitivity::LocalOnly,
        HASH,
        None,
    )
    .unwrap();
    assert!(
        plain.iter().all(|o| o.root_id.is_none()),
        "no registry means no attribution"
    );
    assert!(
        plain.iter().all(|o| o.to_json().get("root_id").is_none()),
        "unattributed occurrences keep the pre-registry wire shape"
    );
}

#[test]
fn sharded_backend_with_registry_serves_attribution_in_get_chunk() {
    // End to end: a registry-attached ShardedBackend reports
    // root_id on the registered occurrence through get-chunk, and
    // omits the key on the unregistered one; without the
    // attachment neither row carries the key.
    let dir = temp_dir("attribchunk");
    fixture(
        &dir,
        "atlas-core-internal-work-01",
        &[("repo@abc:infra/dns/a.yaml", 0, 17)],
        false,
    );
    fixture(
        &dir,
        "atlas-core-internal-media-01",
        &[("repo@abc:infra/dns/b.yaml", 4, 21)],
        false,
    );
    let set = std::sync::Arc::new(discover_set(&dir, Tier::Secret));

    let plain = ShardedBackend::new(Monolith { chunk_id: 7 }, set.clone());
    let out = run_get_chunk(&plain, 7, 0).unwrap();
    let occ = out.get("occurrences").unwrap().as_array().unwrap();
    assert!(
        occ.iter().all(|o| o.get("root_id").is_none()),
        "no registry attached: wire shape unchanged"
    );

    let registry = registry_with_one_root();
    let attributed =
        ShardedBackend::new(Monolith { chunk_id: 7 }, set).with_source_registry(registry);
    let out = run_get_chunk(&attributed, 7, 0).unwrap();
    let occ = out
        .get("occurrences")
        .expect("occurrences present")
        .as_array()
        .unwrap();
    assert_eq!(occ.len(), 2);
    let by_source: std::collections::HashMap<&str, &serde_json::Value> = occ
        .iter()
        .map(|o| (o.get("source").and_then(|s| s.as_str()).unwrap(), o))
        .collect();
    assert_eq!(
        by_source["repo@abc:infra/dns/a.yaml"]
            .get("root_id")
            .and_then(|v| v.as_str()),
        Some("repo"),
        "registered occurrence resolves through get-chunk"
    );
    assert!(
        by_source["repo@abc:infra/dns/b.yaml"]
            .get("root_id")
            .is_none(),
        "unregistered occurrence keeps the old wire shape"
    );
}
