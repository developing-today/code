//! Oracle tests for the Atlas zettelkasten notes connector
//! (#8626 slice 4, design `memory-integration-design-8567.md`
//! section 1): a notes vault stages into the zettelkasten unit
//! carrying the design's provenance set; an unchanged re-scan
//! stages zero; and an edited note re-stages exactly its chunks.

use std::path::{Path, PathBuf};

use atlas_core::memory::register_memory_corpora;
use atlas_core::notes::{ZETTELKASTEN_UNIT, scan_and_stage_note_files_in_db, scan_note_files};
use atlas_core::stage::{Registry, list_staged_items_in_db};

const NOW: &str = "2026-11-24T00:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-notes-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

/// A two-note vault: one top-level note with wikilinks and tags,
/// one nested note, plus non-markdown files that are not corpus
/// content.
fn write_vault(root: &Path) {
    std::fs::create_dir_all(root.join("habits")).unwrap();
    std::fs::write(
        root.join("sleep.md"),
        "# Sleep\n\nLights out by 23:00. See [[habits/recovery]] for rest days. #health\n",
    )
    .unwrap();
    std::fs::write(
        root.join("habits/recovery.md"),
        "# Recovery\n\nOne full rest day per week. Links back to [[sleep]].\n",
    )
    .unwrap();
    std::fs::write(root.join("cover.png"), b"\x89PNG not a note").unwrap();
    std::fs::write(root.join("index.html"), "<p>not a note</p>").unwrap();
}

fn register_zettelkasten(dir: &Path) -> (Registry, PathBuf) {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, dir, NOW).unwrap();
    let db = units
        .iter()
        .find(|u| u.unit_id == ZETTELKASTEN_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .expect("zettelkasten unit registered");
    (registry, db)
}

/// Oracle: the vault scan stages every markdown note and nothing
/// else, each item carrying the design's provenance set (corpus,
/// null agent, source_ref, mtime updated_at, sensitivity,
/// zone=fleet).
#[test]
fn vault_notes_stage_with_the_design_provenance_set() {
    let dir = temp_dir("provenance");
    let vault = dir.join("vault");
    write_vault(&vault);
    let (_registry, db) = register_zettelkasten(&dir);

    let scanned = scan_note_files(&vault).unwrap();
    assert_eq!(
        scanned
            .iter()
            .map(|f| f.rel_path.as_str())
            .collect::<Vec<_>>(),
        vec!["habits/recovery.md", "sleep.md"],
        "scan order is sorted tree-relative"
    );

    let ids = scan_and_stage_note_files_in_db(&db, ZETTELKASTEN_UNIT, &vault, NOW).unwrap();
    assert_eq!(ids.len(), 2, "both notes stage");

    let items = list_staged_items_in_db(&db, Some(ZETTELKASTEN_UNIT)).unwrap();
    assert_eq!(items.len(), 2, "no non-markdown content staged");
    assert_eq!(items[0].kind, "note");
    assert_eq!(items[0].source.as_deref(), Some("note:habits/recovery.md"));
    assert_eq!(items[0].sensitivity, "internal");
    assert_eq!(items[0].chunk_count, 1);

    let meta: serde_json::Value =
        serde_json::from_str(items[0].meta.as_deref().expect("meta recorded")).unwrap();
    assert_eq!(meta["corpus"], "zettelkasten");
    assert!(meta["agent"].is_null(), "the vault records no author");
    assert_eq!(meta["source_ref"], "note:habits/recovery.md");
    assert_eq!(
        meta["updated_at"],
        scanned[0].updated_at.as_deref().expect("mtime recorded"),
        "updated_at is the file's mtime"
    );
    assert_eq!(meta["sensitivity"], "internal");
    assert_eq!(meta["zone"], "fleet");

    // The note text stages whole, wikilinks included.
    let sleep = items
        .iter()
        .find(|i| i.source.as_deref() == Some("note:sleep.md"))
        .unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    let chunk: String = conn
        .query_row(
            "SELECT text FROM chunks_raw WHERE item_id = ?1 ORDER BY idx",
            [sleep.id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        chunk.contains("[[habits/recovery]]"),
        "links ride along as text"
    );
}

/// Oracle: the slice's incremental acceptance — re-scanning an
/// unchanged vault stages zero, and editing one note re-stages
/// exactly its chunks (its item updated in place, every other item
/// untouched).
#[test]
fn unchanged_rescan_stages_zero_and_an_edit_restages_one_note() {
    let dir = temp_dir("restage");
    let vault = dir.join("vault");
    write_vault(&vault);
    let (_registry, db) = register_zettelkasten(&dir);

    let first = scan_and_stage_note_files_in_db(&db, ZETTELKASTEN_UNIT, &vault, NOW).unwrap();
    assert_eq!(first.len(), 2);

    let again = scan_and_stage_note_files_in_db(&db, ZETTELKASTEN_UNIT, &vault, NOW).unwrap();
    assert!(again.is_empty(), "unchanged re-scan stages zero");
    let items = list_staged_items_in_db(&db, Some(ZETTELKASTEN_UNIT)).unwrap();
    assert_eq!(items.len(), 2, "no duplicate items");

    // Edit one note: only its item re-stages.
    std::fs::write(
        vault.join("sleep.md"),
        "# Sleep\n\nLights out by 22:30 now. See [[habits/recovery]] for rest days. #health\n",
    )
    .unwrap();
    let edited = scan_and_stage_note_files_in_db(&db, ZETTELKASTEN_UNIT, &vault, NOW).unwrap();
    assert_eq!(edited.len(), 1, "exactly the edited note re-stages");
    assert_eq!(edited[0], first[1], "the note's item is updated in place");

    let items = list_staged_items_in_db(&db, Some(ZETTELKASTEN_UNIT)).unwrap();
    assert_eq!(items.len(), 2, "still exactly two items");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let texts: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare("SELECT item_id, text FROM chunks_raw ORDER BY item_id, idx")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        texts.len(),
        2,
        "one chunk per note, replaced not duplicated"
    );
    let sleep_chunk = texts
        .iter()
        .find(|(id, _)| *id == first[1])
        .map(|(_, t)| t.as_str())
        .unwrap();
    assert!(
        sleep_chunk.contains("22:30"),
        "the edited note's chunk is the new text"
    );
    let recovery_chunk = texts
        .iter()
        .find(|(id, _)| *id == first[0])
        .map(|(_, t)| t.as_str())
        .unwrap();
    assert!(
        recovery_chunk.contains("One full rest day"),
        "the untouched note's chunk is unchanged"
    );
}

/// Oracle: vault-shape errors are loud — a missing vault root names
/// its path, and non-markdown files are skipped, never staged.
#[test]
fn vault_shape_errors_are_loud_and_non_notes_are_skipped() {
    let dir = temp_dir("shape");
    let err = scan_note_files(&dir.join("no-such-vault"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no-such-vault"), "error names the path: {err}");

    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("binary.png"), b"\x89PNG").unwrap();
    let scanned = scan_note_files(&vault).unwrap();
    assert!(scanned.is_empty(), "non-markdown files are not notes");
}
