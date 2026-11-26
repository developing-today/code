//! Oracle tests for the Atlas research-reports connector
//! (#8626 slice 4, design `memory-integration-design-8567.md`
//! sections 1 and 5): a tree of fleet research outputs (goal files,
//! briefs) stages into the research-reports unit carrying the
//! design's provenance set; re-scanning an unchanged tree stages
//! nothing; editing one report re-stages exactly that report.

use std::path::{Path, PathBuf};

use atlas_core::memory::register_memory_corpora;
use atlas_core::reports::{
    RESEARCH_REPORTS_UNIT, collect_research_reports, stage_research_reports_in_db,
};
use atlas_core::stage::{Registry, list_staged_items_in_db};

const NOW: &str = "2026-11-24T00:00:00Z";
const LATER: &str = "2026-11-24T01:00:00Z";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-reports-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write report");
}

/// A small research-output tree: two Markdown reports (one nested
/// under a goal's briefs directory), plus files the connector must
/// not ingest — a non-Markdown file and a hidden file.
fn report_tree(dir: &Path) -> PathBuf {
    let tree = dir.join("tree");
    write(
        &tree.join("atlas-brief.md"),
        "# Atlas brief\n\nThe unified index wins over per-agent silos.\n",
    );
    write(
        &tree.join("goals/fleet/briefs/dispatch-report.md"),
        "# Dispatch report\n\nRemote dispatch kept the laptop idle all week.\n",
    );
    write(&tree.join("notes.txt"), "not a report\n");
    write(&tree.join(".hidden.md"), "hidden state, not a report\n");
    tree
}

fn reports_db(dir: &Path) -> PathBuf {
    let registry = Registry::open(&dir.join("registry.db")).unwrap();
    let units = register_memory_corpora(&registry, dir, NOW).unwrap();
    units
        .iter()
        .find(|u| u.unit_id == RESEARCH_REPORTS_UNIT)
        .map(|u| PathBuf::from(&u.db_uri))
        .expect("research-reports unit registered")
}

/// Oracle: the staged items after collecting a report tree — one
/// item per Markdown report, each carrying the design's full
/// provenance set (corpus, source_ref, updated_at from the file,
/// sensitivity, zone=fleet), and nothing for non-report files.
#[test]
fn a_report_tree_stages_with_full_provenance() {
    let dir = temp_dir("provenance");
    let db = reports_db(&dir);
    let tree = report_tree(&dir);

    let reports = collect_research_reports(&tree).unwrap();
    let rel: Vec<&str> = reports.iter().map(|r| r.rel_path.as_str()).collect();
    assert_eq!(
        rel,
        vec!["atlas-brief.md", "goals/fleet/briefs/dispatch-report.md"],
        "only the tree's Markdown reports are collected, in path order"
    );

    let staged = stage_research_reports_in_db(&db, RESEARCH_REPORTS_UNIT, &reports, NOW).unwrap();
    assert_eq!(staged.len(), 2);

    let items = list_staged_items_in_db(&db, Some(RESEARCH_REPORTS_UNIT)).unwrap();
    assert_eq!(items.len(), 2);
    for item in &items {
        assert_eq!(item.kind, "research-report");
        assert_eq!(item.sensitivity, "internal");
        let source = item.source.as_deref().expect("source pointer");
        assert!(source.starts_with("research-reports:"), "source {source}");
        let meta: serde_json::Value =
            serde_json::from_str(item.meta.as_deref().expect("provenance meta")).unwrap();
        assert_eq!(meta["corpus"], "research-reports");
        assert_eq!(meta["source_ref"], source);
        assert_eq!(meta["zone"], "fleet");
        assert_eq!(meta["sensitivity"], "internal");
        assert!(
            meta["updated_at"].is_string(),
            "updated_at provenance comes from the source file: {meta}"
        );
        assert!(meta["agent"].is_null(), "agent is carried only where known");
        assert_eq!(item.chunk_count, 1, "a short report is one chunk");
    }
    let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("unified index wins")));
    assert!(texts.iter().any(|t| t.contains("laptop idle")));
}

/// Oracle: the incremental re-index #8626's acceptance asks for,
/// for the research-reports family — re-staging an unchanged tree
/// stages nothing and changes no item; editing one report
/// re-stages exactly that report (its id is the only one returned,
/// its text is the new text, the other report is untouched).
#[test]
fn an_edited_report_restages_exactly_its_own_chunks() {
    let dir = temp_dir("incremental");
    let db = reports_db(&dir);
    let tree = report_tree(&dir);

    let reports = collect_research_reports(&tree).unwrap();
    let first = stage_research_reports_in_db(&db, RESEARCH_REPORTS_UNIT, &reports, NOW).unwrap();
    assert_eq!(first.len(), 2);
    let before = list_staged_items_in_db(&db, Some(RESEARCH_REPORTS_UNIT)).unwrap();

    // Unchanged re-scan: nothing staged, nothing changed.
    let reports = collect_research_reports(&tree).unwrap();
    let restaged =
        stage_research_reports_in_db(&db, RESEARCH_REPORTS_UNIT, &reports, LATER).unwrap();
    assert!(restaged.is_empty(), "unchanged tree re-stages nothing");
    let after = list_staged_items_in_db(&db, Some(RESEARCH_REPORTS_UNIT)).unwrap();
    assert_eq!(before.len(), after.len());
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.id, a.id);
        assert_eq!(b.text, a.text);
        assert_eq!(b.chunk_count, a.chunk_count);
        assert_eq!(
            b.meta, a.meta,
            "an unchanged re-scan rewrites no provenance"
        );
    }

    // Edit one report: exactly its item is re-staged.
    let edited_path = tree.join("atlas-brief.md");
    write(
        &edited_path,
        "# Atlas brief\n\nRevised: the unified index wins, with evidence appended.\n",
    );
    let reports = collect_research_reports(&tree).unwrap();
    let restaged =
        stage_research_reports_in_db(&db, RESEARCH_REPORTS_UNIT, &reports, LATER).unwrap();
    let edited_id = before
        .iter()
        .find(|i| i.source.as_deref() == Some("research-reports:atlas-brief.md"))
        .map(|i| i.id)
        .expect("brief item");
    assert_eq!(
        restaged,
        vec![edited_id],
        "only the edited report re-stages"
    );

    let after = list_staged_items_in_db(&db, Some(RESEARCH_REPORTS_UNIT)).unwrap();
    for item in &after {
        if item.id == edited_id {
            assert!(item.text.contains("with evidence appended"));
        } else {
            let old = before.iter().find(|b| b.id == item.id).unwrap();
            assert_eq!(item.text, old.text, "the unedited report is untouched");
            assert_eq!(item.meta, old.meta);
        }
    }
}

/// Oracle: malformed sources fail loudly — a non-UTF-8 Markdown
/// file and a missing tree are errors naming the problem, never a
/// silently partial ingest.
#[test]
fn malformed_report_sources_fail_loudly() {
    let dir = temp_dir("loud");
    let tree = dir.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("bad.md"), [0xff, 0xfe, 0x00]).unwrap();
    let err = collect_research_reports(&tree).unwrap_err();
    assert!(
        err.to_string().contains("bad.md"),
        "error names the malformed report: {err}"
    );

    let err = collect_research_reports(&dir.join("missing")).unwrap_err();
    assert!(
        err.to_string().contains("not a directory"),
        "error names the missing tree: {err}"
    );
}
