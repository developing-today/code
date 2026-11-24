//! Oracle tests for the Atlas memory-integration connector
//! (#8622 slice 1a, design `memory-integration-design-8567.md`
//! sections 1–2): the five design corpora register in the stage
//! registry with provenance; a memory-lancedb export stages into
//! the agent-memory unit carrying the design's provenance set; and
//! promotion through the aggregator makes the staged memories
//! searchable by the production fan-out, each hit carrying its
//! source-of-truth pointer.

use std::path::{Path, PathBuf};

use atlas_core::aggregate::embed_unit_into_shard;
use atlas_core::embed::DocEmbedder;
use atlas_core::error::Result;
use atlas_core::filters::Filters;
use atlas_core::memory::{
    AGENT_MEMORY_UNIT, MEMORY_CORPORA, parse_memory_export, register_memory_corpora,
    stage_memory_export_in_db,
};
use atlas_core::shards::scan::{FanoutQuery, fanout_scan};
use atlas_core::shards::{Tier, TierCeiling, open_from_spec};
use atlas_core::stage::{Registry, UnitState, list_staged_items_in_db};

const NOW: &str = "2026-11-23T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-memory-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// A read-only export of the memory-lancedb store, in the store's
/// own column shape (the `vector` column included, as the store
/// would emit it — the connector must ignore it).
const EXPORT: &str = r#"[
  {
    "id": "mem-transcript",
    "text": "Continue the current task from the existing transcript, preserving completed work.",
    "vector": [0.25, 0.5, 0.75],
    "importance": 0.9,
    "category": "fact",
    "createdAt": 1763000000000.0,
    "agentId": "main"
  },
  {
    "id": "mem-deploy",
    "text": "The deploy window opens on Sunday evening after the freeze lifts.",
    "vector": [0.1, 0.2, 0.3],
    "importance": 0.4,
    "category": "fact",
    "createdAt": 1763100000000.0,
    "agentId": "main"
  }
]"#;

/// A document embedder keyed on content: transcript text lands on
/// the first axis, everything else on the second — enough structure
/// for the fan-out oracle to rank by (mirrors the aggregator oracle
/// tests).
struct KeywordEmbedder;

impl DocEmbedder for KeywordEmbedder {
    fn embed_doc(&self, text: &str) -> Result<Vec<f32>> {
        if text.contains("transcript") {
            Ok(vec![1.0, 0.0, 0.0])
        } else {
            Ok(vec![0.0, 1.0, 0.0])
        }
    }
}

fn register(dir: &Path) -> (Registry, PathBuf) {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, dir, NOW).unwrap();
    let agent_db = units
        .iter()
        .find(|u| u.unit_id == AGENT_MEMORY_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .expect("agent-memory unit registered");
    (registry, agent_db)
}

/// Oracle: the registry contents after registration — exactly the
/// design's five corpora, each open, each carrying its owner and
/// tier as provenance, each with its staging DB on disk; and a
/// second registration changes nothing (idempotent).
#[test]
fn the_five_design_corpora_register_with_provenance() {
    let dir = temp_dir("corpora");
    let registry = Registry::open(&dir.join("registry.db")).unwrap();

    let units = register_memory_corpora(&registry, &dir, NOW).unwrap();
    assert_eq!(units.len(), 5);
    let ids: Vec<&str> = units.iter().map(|u| u.unit_id.as_str()).collect();
    let want: Vec<&str> = MEMORY_CORPORA.iter().map(|c| c.unit_id).collect();
    assert_eq!(ids, want);
    for (unit, corpus) in units.iter().zip(MEMORY_CORPORA.iter()) {
        assert_eq!(
            unit.owner, corpus.owner,
            "owner provenance for {}",
            unit.unit_id
        );
        assert_eq!(unit.sensitivity, corpus.sensitivity);
        assert_eq!(unit.state, UnitState::Open);
        assert!(
            Path::new(&unit.db_uri).exists(),
            "staging DB for {} exists",
            unit.unit_id
        );
    }

    let again = register_memory_corpora(&registry, &dir, NOW).unwrap();
    assert_eq!(again, units, "re-registration returns the recorded units");
    assert_eq!(
        registry.units_in_state(UnitState::Open).unwrap().len(),
        5,
        "still exactly five open units"
    );
}

/// Oracle: the export parser's field mapping — the store's columns
/// land on the entry, the store's vector is not carried anywhere,
/// and a malformed row fails loudly, naming the row and field.
#[test]
fn export_parsing_reads_the_store_columns_and_ignores_its_vectors() {
    let entries = parse_memory_export(EXPORT).unwrap();
    assert_eq!(entries.len(), 2);
    let first = &entries[0];
    assert_eq!(first.id, "mem-transcript");
    assert_eq!(first.agent.as_deref(), Some("main"));
    assert_eq!(first.category.as_deref(), Some("fact"));
    assert_eq!(first.importance, Some(0.9));
    assert_eq!(first.created_at_ms, Some(1_763_000_000_000.0));
    assert_eq!(first.sensitivity, Tier::Internal);
    assert_eq!(
        first.source_ref(),
        "memory-lancedb:mem-transcript",
        "the source-of-truth pointer names the store and row"
    );
    assert_eq!(
        first.updated_at().as_deref(),
        Some("2025-11-13T02:13:20Z"),
        "createdAt (ms epoch) becomes the RFC 3339 updated_at"
    );

    let missing_text = r#"[{"id": "ok", "text": "fine"}, {"id": "broken"}]"#;
    let err = parse_memory_export(missing_text).unwrap_err().to_string();
    assert!(err.contains("row 1"), "error names the row: {err}");
    assert!(err.contains("`text`"), "error names the field: {err}");

    let not_an_array = r#"{"id": "x", "text": "y"}"#;
    assert!(parse_memory_export(not_an_array).is_err());
}

/// Oracle: a staged item's recorded provenance — the items row and
/// its meta carry the design's full provenance set (corpus, agent,
/// source_ref, updated_at, sensitivity, zone=fleet), and the chunk
/// rows partition the memory text.
#[test]
fn staged_memory_carries_the_design_provenance_set() {
    let dir = temp_dir("provenance");
    let (_registry, agent_db) = register(&dir);
    let entries = parse_memory_export(EXPORT).unwrap();
    let ids = stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    assert_eq!(ids.len(), 2);

    let items = list_staged_items_in_db(&agent_db, Some(AGENT_MEMORY_UNIT)).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, "memory");
    assert_eq!(
        items[0].source.as_deref(),
        Some("memory-lancedb:mem-transcript")
    );
    assert_eq!(items[0].sensitivity, "internal");
    assert_eq!(items[0].chunk_count, 1);

    let meta: serde_json::Value =
        serde_json::from_str(items[0].meta.as_deref().expect("meta recorded")).unwrap();
    assert_eq!(meta["corpus"], "agent-memory");
    assert_eq!(meta["agent"], "main");
    assert_eq!(meta["source_ref"], "memory-lancedb:mem-transcript");
    assert_eq!(meta["updated_at"], "2025-11-13T02:13:20Z");
    assert_eq!(meta["sensitivity"], "internal");
    assert_eq!(meta["zone"], "fleet");
    assert_eq!(meta["category"], "fact");
    assert_eq!(meta["lancedb_id"], "mem-transcript");
}

/// Oracle: the slice's acceptance — after promotion through the
/// aggregator (embed-at-promotion under the mandated model gate),
/// the production fan-out returns the agent-memory hit for a
/// matching query, the shard chunk carries the memory's
/// source-of-truth pointer, and the unit ends merged with its
/// merge manifest recorded.
#[test]
fn promoted_agent_memory_is_searchable_by_the_fanout() {
    let dir = temp_dir("search");
    let (registry, agent_db) = register(&dir);
    let entries = parse_memory_export(EXPORT).unwrap();
    stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();

    let shard_dir = dir.join("shards");
    let report = embed_unit_into_shard(
        &registry,
        AGENT_MEMORY_UNIT,
        &shard_dir,
        "agent-memory",
        &KeywordEmbedder,
        NOW,
    )
    .unwrap();
    assert_eq!(report.items, 2);
    assert_eq!(report.chunks, 2);
    assert_eq!(report.shard_name, "atlas-core-internal-agent-memory-00");
    assert_eq!(
        registry.lookup(AGENT_MEMORY_UNIT).unwrap().unwrap().state,
        UnitState::Merged
    );
    assert!(
        registry
            .merge_manifest(AGENT_MEMORY_UNIT)
            .unwrap()
            .is_some(),
        "the promotion's merge manifest is the durable record"
    );

    // The shard chunk carries the source-of-truth pointer, so a hit
    // can always be opened at its source (design section 3).
    let conn = rusqlite::Connection::open(&report.shard_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT message_id, text FROM chunks ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows[0].0, "memory-lancedb:mem-transcript");
    assert!(rows[0].1.contains("transcript"));

    // The fan-out ranks the transcript memory first for a
    // transcript-shaped query vector.
    let set = open_from_spec(
        &format!("dir:{}", shard_dir.display()),
        TierCeiling(Tier::Internal),
        atlas_core::DEFAULT_VEC_TABLE,
    )
    .unwrap();
    assert_eq!(set.shards().len(), 1);
    let filters = Filters::new(vec![], vec![], vec![], vec![], "personal", None, None).unwrap();
    let hits = fanout_scan(
        &set,
        &FanoutQuery {
            qvec: &[1.0, 0.0, 0.0],
            shard_top_k: 10,
            global_top_k: 10,
            filters: &filters,
            topics: None,
            ceiling: TierCeiling(Tier::Internal),
        },
    )
    .unwrap()
    .hits;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].chunk_id, 1, "the transcript memory ranks first");
    assert!(hits[0].dist < hits[1].dist);
}

/// Write a synthetic memory-lancedb-shaped dataset (the store's own
/// columns, including a vector the reader must ignore) into `path`.
fn write_fixture_store(path: &Path) {
    use std::sync::Arc;
    use arrow::array::{FixedSizeListArray, Float64Array, RecordBatch, RecordBatchIterator, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    let vec_field = Arc::new(Field::new("item", DataType::Float32, true));
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("vector", DataType::FixedSizeList(vec_field.clone(), 3), true),
        Field::new("importance", DataType::Float64, true),
        Field::new("category", DataType::Utf8, true),
        Field::new("createdAt", DataType::Float64, true),
        Field::new("agentId", DataType::Utf8, true),
    ]));
    let values = arrow::array::Float32Array::from(vec![0.25f32, 0.5, 0.75, 0.1, 0.2, 0.3]);
    let vectors = FixedSizeListArray::new(vec_field, 3, Arc::new(values), None);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["mem-transcript", "mem-deploy"])),
            Arc::new(StringArray::from(vec![
                "Continue the current task from the existing transcript, preserving completed work.",
                "The deploy window opens on Sunday evening after the freeze lifts.",
            ])),
            Arc::new(vectors),
            Arc::new(Float64Array::from(vec![0.9, 0.4])),
            Arc::new(StringArray::from(vec!["fact", "fact"])),
            Arc::new(Float64Array::from(vec![1_763_000_000_000.0, 1_763_100_000_000.0])),
            Arc::new(StringArray::from(vec!["main", "main"])),
        ],
    )
    .unwrap();
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    let uri = path.to_str().unwrap().to_string();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        lance::Dataset::write(reader, &uri, None).await.unwrap();
    });
}

/// Oracle: the native reader maps a `.lance` dataset's columns onto
/// the same entries the JSON export parser produces, and never reads
/// the vector column.
#[test]
fn native_reader_reads_a_lancedb_store_without_its_vectors() {
    let dir = temp_dir("native");
    let store = dir.join("memories.lance");
    write_fixture_store(&store);

    let mut native = atlas_core::memory::read_lancedb_store_blocking(&store).unwrap();
    native.sort_by(|a, b| a.id.cmp(&b.id));
    let mut json = parse_memory_export(EXPORT).unwrap();
    json.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(native, json);
}

/// Oracle: rebuild-from-sources — staging the native read into a
/// fresh unit and promoting it yields a searchable shard with the
/// same hits as the export path (the staged corpus is the source).
#[test]
fn native_store_rebuilds_the_index_from_sources() {
    let dir = temp_dir("rebuild");
    let store = dir.join("memories.lance");
    write_fixture_store(&store);
    let (registry, agent_db) = register(&dir);
    let entries = atlas_core::memory::read_lancedb_store_blocking(&store).unwrap();
    stage_memory_export_in_db(&agent_db, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    let report = embed_unit_into_shard(
        &registry, AGENT_MEMORY_UNIT, &dir.join("shards"), "agent-memory", &KeywordEmbedder, NOW,
    )
    .unwrap();
    assert_eq!(report.items, 2);
    assert_eq!(report.chunks, 2);
}

/// Oracle: idempotency — a second sync of an unchanged store restages
/// nothing; a changed text restages only that entry.
#[test]
fn sync_is_idempotent_and_restages_only_changed_entries() {
    let dir = temp_dir("sync");
    let (_registry, agent_db) = register(&dir);
    let conn = rusqlite::Connection::open(&agent_db).unwrap();
    let mut entries = parse_memory_export(EXPORT).unwrap();

    let first = atlas_core::memory::sync_memory_entries(&conn, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    assert_eq!((first.added, first.restaged, first.unchanged), (2, 0, 0));

    let again = atlas_core::memory::sync_memory_entries(&conn, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    assert_eq!((again.added, again.restaged, again.unchanged), (0, 0, 2));

    entries[1].text = "The deploy window moved to Monday.".to_string();
    let changed = atlas_core::memory::sync_memory_entries(&conn, AGENT_MEMORY_UNIT, &entries, NOW).unwrap();
    assert_eq!((changed.added, changed.restaged, changed.unchanged), (0, 1, 1));

    let items = list_staged_items_in_db(&agent_db, Some(AGENT_MEMORY_UNIT)).unwrap();
    assert_eq!(items.len(), 2, "restaging replaces the old item");
}
