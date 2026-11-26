//! Oracle tests for the fleet-memory-files connector (#8626,
//! slice 4's fleet-memory half, design
//! `memory-integration-design-8567.md` sections 1–2): the Muse
//! memory tree (`MEMORY.md`, topic files, daily logs) scans into
//! the `fleet-memory` unit carrying the design's provenance set,
//! and re-scans are incremental — an unchanged tree stages zero,
//! an edited file re-stages exactly its chunks.

use std::path::{Path, PathBuf};

use atlas_core::memory::{FLEET_MEMORY_UNIT, scan_memory_files, stage_memory_files_in_db};
use atlas_core::stage::{Registry, list_staged_items_in_db};

const NOW: &str = "2026-11-24T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-memory-files-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn write_tree(root: &Path) {
    std::fs::write(
        root.join("MEMORY.md"),
        "# Standing memory\n\nThe deploy window opens on Sunday evening.\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("topics")).unwrap();
    std::fs::write(
        root.join("topics").join("deploy.md"),
        "# Deploy topic\n\nFreeze lifts before the window opens.\n",
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "not corpus content\n").unwrap();
}

fn register(dir: &Path) -> PathBuf {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = atlas_core::memory::register_memory_corpora(&registry, dir, NOW).unwrap();
    units
        .iter()
        .find(|u| u.unit_id == FLEET_MEMORY_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .expect("fleet-memory unit registered")
}

/// Oracle: the scan finds every markdown file in sorted
/// tree-relative order and skips non-markdown files.
#[test]
fn scan_finds_the_markdown_tree_in_sorted_order() {
    let dir = temp_dir("scan");
    let root = dir.join("tree");
    std::fs::create_dir_all(&root).unwrap();
    write_tree(&root);

    let files = scan_memory_files(&root).unwrap();
    let rels: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
    assert_eq!(rels, vec!["MEMORY.md", "topics/deploy.md"]);
    assert!(
        files.iter().all(|f| f.updated_at.is_some()),
        "mtime-backed updated_at is recorded"
    );
}

/// Oracle: a missing tree root is a loud error, never an empty
/// ingest.
#[test]
fn scan_of_a_missing_root_fails_loudly() {
    let dir = temp_dir("missing");
    assert!(scan_memory_files(&dir.join("no-such-tree")).is_err());
}

/// Oracle: a staged file's recorded provenance — the items row
/// and its meta carry the design's full provenance set (corpus
/// `fleet-memory`, source_ref `memory-file:<rel>`, updated_at,
/// sensitivity, zone=fleet), and the chunk rows partition the
/// file text.
#[test]
fn staged_memory_files_carry_the_design_provenance_set() {
    let dir = temp_dir("provenance");
    let root = dir.join("tree");
    std::fs::create_dir_all(&root).unwrap();
    write_tree(&root);
    let db = register(&dir);

    let files = scan_memory_files(&root).unwrap();
    let ids = stage_memory_files_in_db(&db, FLEET_MEMORY_UNIT, &files, NOW).unwrap();
    assert_eq!(ids.len(), 2);

    let items = list_staged_items_in_db(&db, Some(FLEET_MEMORY_UNIT)).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind, "memory");
    assert_eq!(items[0].source.as_deref(), Some("memory-file:MEMORY.md"));
    assert_eq!(items[0].sensitivity, "internal");
    assert_eq!(items[0].chunk_count, 1);

    let meta: serde_json::Value =
        serde_json::from_str(items[0].meta.as_deref().expect("meta recorded")).unwrap();
    assert_eq!(meta["corpus"], "fleet-memory");
    assert!(meta["agent"].is_null(), "the tree records no author");
    assert_eq!(meta["source_ref"], "memory-file:MEMORY.md");
    assert_eq!(meta["sensitivity"], "internal");
    assert_eq!(meta["zone"], "fleet");
    assert_eq!(meta["rel_path"], "MEMORY.md");
    assert!(
        meta["updated_at"].is_string(),
        "mtime-backed updated_at: {meta}"
    );
}

/// Oracle: replaying the same tree revision is a no-op, reported
/// by the public files staging seam as zero changed item ids.
#[test]
fn rescanning_an_unchanged_tree_stages_zero() {
    let dir = temp_dir("idempotent");
    let root = dir.join("tree");
    std::fs::create_dir_all(&root).unwrap();
    write_tree(&root);
    let db = register(&dir);

    let first = stage_memory_files_in_db(
        &db,
        FLEET_MEMORY_UNIT,
        &scan_memory_files(&root).unwrap(),
        NOW,
    )
    .unwrap();
    let second = stage_memory_files_in_db(
        &db,
        FLEET_MEMORY_UNIT,
        &scan_memory_files(&root).unwrap(),
        "2026-11-25T00:00:00Z",
    )
    .unwrap();

    assert_eq!(first.len(), 2);
    assert!(
        second.is_empty(),
        "an unchanged tree revision stages zero changed items"
    );
    assert_eq!(
        list_staged_items_in_db(&db, Some(FLEET_MEMORY_UNIT))
            .unwrap()
            .len(),
        2
    );
}

/// Oracle: editing one file re-stages exactly its chunks under
/// the same source identity instead of duplicating, and leaves
/// the untouched file's item alone.
#[test]
fn an_edited_file_restages_exactly_its_chunks() {
    let dir = temp_dir("revision");
    let root = dir.join("tree");
    std::fs::create_dir_all(&root).unwrap();
    write_tree(&root);
    let db = register(&dir);

    let initial = stage_memory_files_in_db(
        &db,
        FLEET_MEMORY_UNIT,
        &scan_memory_files(&root).unwrap(),
        NOW,
    )
    .unwrap();
    let untouched_id = initial[1];

    std::fs::write(
        root.join("MEMORY.md"),
        "# Standing memory\n\nThe deploy window opens on Sunday evening after the freeze lifts.\nA second line keeps the file multi-chunk only if it grows past the ceiling; it does not.\n",
    )
    .unwrap();
    let changed = stage_memory_files_in_db(
        &db,
        FLEET_MEMORY_UNIT,
        &scan_memory_files(&root).unwrap(),
        "2026-11-25T00:01:00Z",
    )
    .unwrap();
    let items = list_staged_items_in_db(&db, Some(FLEET_MEMORY_UNIT)).unwrap();

    assert_eq!(
        changed,
        vec![initial[0]],
        "exactly the edited file re-stages"
    );
    assert_eq!(
        items.len(),
        2,
        "revisions do not duplicate source identities"
    );
    assert_eq!(items[0].id, initial[0]);
    assert_eq!(items[1].id, untouched_id);
    assert!(items[0].text.contains("freeze lifts"));
    assert_eq!(
        items[0].chunk_count, 1,
        "one whole-file chunk is replaced by exactly one chunk"
    );
}
