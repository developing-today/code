//! Export-only degraded mode (#13524 sub-slice 5) acceptance (design
//! §2.5, §5): an export-only run over the corpus asserts its degraded
//! guarantees as such — no page claims full guarantees, every ledger row
//! is named, every page is comment-uncovered with reason `export-path`,
//! and re-import recognises path-keyed pages only by content hash.

use std::collections::BTreeMap;

use anyhow::Result;
use id_import_notion::comments::{CommentsCoverage, convert_comments};
use id_import_notion::content::{ReferenceSurface, convert_page};
use id_import_notion::export::{ExportApi, ExportArchive, ExportFileFetcher, MatchConfidence};
use id_import_notion::export_only::{
    DegradedKey, EXPORT_PATH_REASON, IdentityGuarantee, carry_degraded_identities,
    degradation_headline, export_only_manifest, identity_guarantee,
};
use id_import_notion::{Acquisition, Inventory, Manifest, NOTION_API_VERSION};
use serde_json::json;

const ALPHA_HEX: &str = "11111111111111111111111111111111";
const BETA_HEX: &str = "22222222222222222222222222222222";
const TASKS_HEX: &str = "88888888888888888888888888888888";
const ROW_HEX: &str = "99999999999999999999999999999999";
const ALPHA: &str = "11111111-1111-1111-1111-111111111111";
const BETA: &str = "22222222-2222-2222-2222-222222222222";
const TASKS: &str = "88888888-8888-8888-8888-888888888888";
const ROW: &str = "99999999-9999-9999-9999-999999999999";

fn files(entries: &[(&str, &str)]) -> ExportArchive {
    ExportArchive::from_files(
        entries
            .iter()
            .map(|(path, body)| ((*path).to_owned(), body.as_bytes().to_vec()))
            .collect(),
    )
    .unwrap()
}

fn corpus(alpha_body: &str, loose_path: &str, draft_body: &str) -> ExportArchive {
    let alpha =
        format!("# Alpha\n\n{alpha_body}\n\n[Beta](Alpha%20{ALPHA_HEX}/Beta%20{BETA_HEX}.md)\n");
    let beta = "# Beta\n\nChild text\n".to_owned();
    let csv = "Name,Status\nRow A,open\nRow B,done\n".to_owned();
    let row = "# Row A\n\nStatus: open\n\nRow body\n".to_owned();
    let alpha_path = format!("Alpha {ALPHA_HEX}.md");
    let beta_path = format!("Alpha {ALPHA_HEX}/Beta {BETA_HEX}.md");
    let csv_path = format!("Alpha {ALPHA_HEX}/Tasks {TASKS_HEX}.csv");
    let row_path = format!("Alpha {ALPHA_HEX}/Tasks {TASKS_HEX}/Row A {ROW_HEX}.md");
    files(&[
        (&alpha_path, &alpha),
        (&beta_path, &beta),
        (&csv_path, &csv),
        (&row_path, &row),
        (loose_path, "# Loose\n\nUnchanged loose page\n"),
        ("Draft.md", draft_body),
    ])
}

fn mint_with(prefix: &'static str) -> impl FnMut(&str) -> Result<String> {
    move |page_id: &str| Ok(format!("{prefix}-{page_id}"))
}

#[test]
fn an_export_only_run_asserts_degraded_guarantees() {
    let archive = corpus("First version.", "Loose.md", "# Draft\n\nv1\n");
    let mut manifest = export_only_manifest("workspace-alpha".into(), &archive).unwrap();

    assert_eq!(
        manifest.acquisition,
        Acquisition {
            api: false,
            export: true,
            degraded_mode: true,
        }
    );
    assert_eq!(manifest.acquisition_paths, ["export".to_owned()]);

    // Inventory: 6 pages (Alpha, Beta, Row A, the CSV-only Row B, Loose,
    // Draft) and one database, with the export's folder nesting as parents.
    let pages: Vec<&String> = manifest
        .inventory
        .objects
        .values()
        .filter(|o| o.kind == "page")
        .map(|o| &o.id)
        .collect();
    assert_eq!(pages.len(), 6);
    assert_eq!(manifest.inventory.objects[TASKS].kind, "database");
    assert_eq!(
        manifest.inventory.objects[BETA].parent_id.as_deref(),
        Some(ALPHA)
    );
    assert_eq!(
        manifest.inventory.objects[TASKS].parent_id.as_deref(),
        Some(ALPHA)
    );
    assert_eq!(
        manifest.inventory.objects[ROW].parent_id.as_deref(),
        Some(TASKS)
    );
    let row_b = manifest
        .degraded_identities
        .iter()
        .find(|(_, d)| d.rel_path.ends_with("#row2"))
        .map(|(id, _)| id.clone())
        .unwrap();
    assert_eq!(
        manifest.inventory.objects[&row_b].parent_id.as_deref(),
        Some(TASKS)
    );

    // Every ledger row is export-only, and therefore named.
    assert_eq!(manifest.export_matches.len(), 7);
    for (path, matched) in &manifest.export_matches {
        assert_eq!(matched.confidence, MatchConfidence::ExportOnly, "{path}");
        assert!(matched.needs_naming(), "{path}");
    }

    // No page claims full guarantees; UUID-named and path-keyed differ.
    for page in &pages {
        let guarantee = identity_guarantee(&manifest, page).unwrap();
        assert!(!guarantee.is_full(), "{page}");
    }
    assert_eq!(
        identity_guarantee(&manifest, ALPHA),
        Some(IdentityGuarantee::DegradedExportUuid)
    );
    assert_eq!(
        identity_guarantee(&manifest, &row_b),
        Some(IdentityGuarantee::DegradedContentHashPath)
    );
    assert_eq!(
        manifest.degraded_identities[&row_b].key,
        DegradedKey::ContentHashPath
    );

    // Every page is comment-uncovered with reason export-path.
    for page in &pages {
        assert_eq!(
            manifest.comments_coverage.get(*page),
            Some(&CommentsCoverage::Unavailable),
            "{page}"
        );
        assert_eq!(
            manifest
                .comments_uncovered_reasons
                .get(*page)
                .map(String::as_str),
            Some(EXPORT_PATH_REASON),
            "{page}"
        );
    }

    let headline = degradation_headline(&manifest).unwrap();
    assert!(headline.starts_with("Degraded import"), "{headline}");
    assert!(headline.contains("6 pages"), "{headline}");
    assert!(headline.contains("3 of them keyed by path"), "{headline}");

    // The same pipeline converts the export-only pages.
    assert_eq!(
        manifest
            .assign_page_identities(&mut mint_with("note"))
            .unwrap(),
        6
    );
    let mut api = ExportApi::new(&archive, &manifest).unwrap();
    let alpha = convert_page(
        &mut api,
        &mut ExportFileFetcher::new(&archive),
        &mut manifest,
        ALPHA,
    )
    .unwrap();
    assert_eq!(alpha.content.title, "Alpha");
    assert!(alpha.references.iter().any(|r| {
        r.surface == ReferenceSurface::ChildPage
            && r.target_note_id.as_deref() == Some(format!("note-{BETA}").as_str())
    }));
    let row_page = convert_page(
        &mut api,
        &mut ExportFileFetcher::new(&archive),
        &mut manifest,
        &row_b,
    )
    .unwrap();
    assert!(row_page.content.blocks.is_empty());
    let row_a = convert_page(
        &mut api,
        &mut ExportFileFetcher::new(&archive),
        &mut manifest,
        ROW,
    )
    .unwrap();
    assert_eq!(row_a.content.blocks.len(), 1);

    // Comment import over the export path yields no claims and keeps the
    // page uncovered.
    let comments = convert_comments(&mut api, &mut manifest, ALPHA).unwrap();
    assert_eq!(comments.coverage, CommentsCoverage::Unavailable);
    assert!(comments.claims.is_empty());

    // The degraded records survive a manifest round trip.
    let loaded: Manifest =
        serde_json::from_value(serde_json::to_value(&manifest).unwrap()).unwrap();
    assert_eq!(loaded.degraded_identities, manifest.degraded_identities);
    assert_eq!(
        loaded.comments_uncovered_reasons,
        manifest.comments_uncovered_reasons
    );
}

#[test]
fn re_import_recognises_path_keyed_pages_only_by_content_hash() {
    let first = corpus("First version.", "Loose.md", "# Draft\n\nv1\n");
    let mut before = export_only_manifest("workspace-alpha".into(), &first).unwrap();
    before
        .assign_page_identities(&mut mint_with("old"))
        .unwrap();

    // Alpha's text changed (still UUID-named), Loose moved unchanged,
    // Draft (no UUID) changed, the CSV-only row is unchanged.
    let second = corpus("Second version.", "Archive/Loose.md", "# Draft\n\nv2\n");
    let mut after = export_only_manifest("workspace-alpha".into(), &second).unwrap();
    let report = carry_degraded_identities(&before, &mut after);

    assert_eq!(
        report.by_uuid.get(ALPHA).map(String::as_str),
        Some(format!("old-{ALPHA}").as_str())
    );
    assert_eq!(report.by_uuid.len(), 3, "Alpha, Beta, Row A");

    let path_of = |manifest: &Manifest, suffix: &str| {
        manifest
            .degraded_identities
            .iter()
            .find(|(_, d)| d.rel_path.ends_with(suffix))
            .map(|(id, _)| id.clone())
            .unwrap()
    };
    let row_b = path_of(&after, "#row2");
    assert_eq!(
        report.by_path_and_hash.get(&row_b),
        before.page_id_to_note_id.get(&path_of(&before, "#row2"))
    );

    let loose = path_of(&after, "Archive/Loose.md");
    assert_eq!(
        report.by_hash_moved.get(&loose),
        before.page_id_to_note_id.get(&path_of(&before, "Loose.md"))
    );

    let draft = path_of(&after, "Draft.md");
    assert_eq!(report.unrecognised, [draft.clone()]);
    assert!(!after.page_id_to_note_id.contains_key(&draft));

    // Only the unrecognised page gets a new note; nothing is shared.
    assert_eq!(
        after.assign_page_identities(&mut mint_with("new")).unwrap(),
        1
    );
    assert_eq!(after.page_id_to_note_id[&draft], format!("new-{draft}"));
    let mut notes: Vec<&String> = after.page_id_to_note_id.values().collect();
    notes.sort();
    notes.dedup();
    assert_eq!(notes.len(), after.page_id_to_note_id.len());

    // Re-importing the same export changes nothing.
    let mut again = export_only_manifest("workspace-alpha".into(), &second).unwrap();
    let repeat = carry_degraded_identities(&after, &mut again);
    assert!(repeat.unrecognised.is_empty());
    assert_eq!(again.page_id_to_note_id, after.page_id_to_note_id);
}

#[test]
fn an_api_run_keeps_full_guarantees() {
    let search = json!({"object": "list", "has_more": false, "results": [{
        "object": "page",
        "id": ALPHA,
        "parent": {"type": "workspace", "workspace": true},
        "properties": {"Name": {"id": "title", "type": "title", "title": [
            {"type": "text", "plain_text": "Alpha", "text": {"content": "Alpha"}},
        ]}},
    }]});
    let inventory = Inventory::from_recorded_responses(
        &[search],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([(ALPHA.to_owned(), true)]),
        None,
    )
    .unwrap();
    let manifest = Manifest::new(
        "workspace-alpha".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap();
    assert_eq!(
        identity_guarantee(&manifest, ALPHA),
        Some(IdentityGuarantee::Full)
    );
    assert_eq!(identity_guarantee(&manifest, BETA), None);
    assert_eq!(degradation_headline(&manifest), None);
    assert!(manifest.degraded_identities.is_empty());
}

#[test]
fn colliding_export_identities_are_refused() {
    // Two files naming the same UUID cannot both be that object.
    let archive = files(&[
        (&format!("One {ALPHA_HEX}.md"), "# One\n"),
        (&format!("Two/Copy {ALPHA_HEX}.md"), "# Copy\n"),
    ]);
    assert!(export_only_manifest("workspace-alpha".into(), &archive).is_err());
}
