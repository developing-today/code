//! Slice 4 export reader (#13524 sub-slice 4) acceptance (design §2.5,
//! §2.6, §5): the same corpus acquired through the API and through a
//! Markdown+CSV export converts to the same content, and every export
//! match that is not certain is named in the manifest's match ledger.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::content::{FileFetcher, ReferenceSurface, convert_page};
use id_import_notion::export::{
    ExportApi, ExportArchive, ExportFileFetcher, ExportKind, MatchConfidence, match_export,
    record_export_matches,
};
use id_import_notion::{Acquisition, Inventory, Manifest, NOTION_API_VERSION, NotionApi};
use serde_json::{Value, json};

const ALPHA: &str = "11111111-1111-1111-1111-111111111111";
const BETA: &str = "22222222-2222-2222-2222-222222222222";
const ALPHA_HEX: &str = "11111111111111111111111111111111";
const BETA_HEX: &str = "22222222222222222222222222222222";
const PICTURE: &[u8] = b"\x89PNG fixture picture bytes";

fn span(text: &str, bold: bool, link: Option<&str>) -> Value {
    let mut item = json!({
        "type": "text",
        "plain_text": text,
        "text": {"content": text},
        "annotations": {"bold": bold, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default"},
    });
    if let Some(url) = link {
        item["text"]["link"] = json!({"url": url});
        item["href"] = json!(url);
    }
    item
}

fn block(id: &str, block_type: &str, payload: Value, has_children: bool) -> Value {
    let mut value = json!({
        "object": "block",
        "id": id,
        "type": block_type,
        "has_children": has_children,
    });
    value[block_type] = payload;
    value
}

fn list(results: Vec<Value>) -> Value {
    json!({"object": "list", "results": results, "has_more": false})
}

/// The corpus as the API serves it.
struct CorpusApi {
    children: BTreeMap<String, Value>,
}

impl CorpusApi {
    fn new() -> Self {
        let beta_url = format!("https://www.notion.so/ws/Beta-{BETA_HEX}");
        let alpha_url = format!("https://www.notion.so/ws/Alpha-{ALPHA_HEX}");
        let alpha = vec![
            block(
                "a-h1",
                "heading_1",
                json!({"rich_text": [span("Overview", false, None)]}),
                false,
            ),
            block(
                "a-p",
                "paragraph",
                json!({"rich_text": [
                    span("See ", false, None),
                    span("Beta", true, Some(&beta_url)),
                    span(" and ", false, None),
                    span("Example", false, Some("https://example.com/x")),
                ]}),
                false,
            ),
            block(
                "a-list",
                "bulleted_list_item",
                json!({"rich_text": [span("Item", false, None)]}),
                true,
            ),
            block(
                "a-todo",
                "to_do",
                json!({"rich_text": [span("Do it", false, None)], "checked": true}),
                false,
            ),
            block(
                "a-num",
                "numbered_list_item",
                json!({"rich_text": [span("First", false, None)]}),
                false,
            ),
            block(BETA, "child_page", json!({"title": "Beta"}), true),
            block(
                "a-img",
                "image",
                json!({
                    "type": "file",
                    "file": {"url": "https://files.notion.so/pic?sig=fixture-secret", "expiry_time": "2026-11-24T00:00:00Z"},
                    "caption": [span("Pic", false, None)],
                }),
                false,
            ),
            block(
                "a-code",
                "code",
                json!({"rich_text": [span("fn main() {}", false, None)], "language": "rust"}),
                false,
            ),
            block(
                "a-quote",
                "quote",
                json!({"rich_text": [span("Quoted", false, None)]}),
                false,
            ),
            block("a-div", "divider", json!({}), false),
        ];
        let children = BTreeMap::from([
            (ALPHA.to_owned(), list(alpha)),
            (
                "a-list".to_owned(),
                list(vec![block(
                    "a-nested",
                    "paragraph",
                    json!({"rich_text": [span("Nested", false, None)]}),
                    false,
                )]),
            ),
            (
                BETA.to_owned(),
                list(vec![block(
                    "b-p",
                    "paragraph",
                    json!({"rich_text": [
                        span("Back to ", false, None),
                        span("Alpha", false, Some(&alpha_url)),
                    ]}),
                    false,
                )]),
            ),
        ]);
        Self { children }
    }
}

impl NotionApi for CorpusApi {
    fn notion_version(&self) -> &str {
        NOTION_API_VERSION
    }
    fn search(&mut self, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn block_children(&mut self, block_id: &str, _cursor: Option<&str>) -> Result<Value> {
        self.children
            .get(block_id)
            .cloned()
            .with_context(|| format!("unexpected block parent {block_id}"))
    }
    fn database(&mut self, _database_id: &str) -> Result<Value> {
        bail!("unused")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        Ok(false)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![ALPHA.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        None
    }
}

struct CdnFetcher;

impl FileFetcher for CdnFetcher {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        if url.contains("files.notion.so/pic") {
            Ok(PICTURE.to_vec())
        } else {
            bail!("unexpected fetch {url}")
        }
    }
}

/// The same corpus as Notion's Markdown+CSV export writes it.
fn corpus_export() -> ExportArchive {
    let alpha_md = format!(
        "# Alpha\n\
         \n\
         # Overview\n\
         \n\
         See **[Beta](Alpha%20{ALPHA_HEX}/Beta%20{BETA_HEX}.md)** and [Example](https://example.com/x)\n\
         \n\
         - Item\n\
         \x20   \n\
         \x20   Nested\n\
         - [x] Do it\n\
         1. First\n\
         \n\
         [Beta](Alpha%20{ALPHA_HEX}/Beta%20{BETA_HEX}.md)\n\
         \n\
         ![Pic](Alpha%20{ALPHA_HEX}/pic.png)\n\
         \n\
         ```rust\n\
         fn main() {{}}\n\
         ```\n\
         \n\
         > Quoted\n\
         \n\
         ---\n"
    );
    let beta_md = format!("# Beta\n\nBack to [Alpha](../Alpha%20{ALPHA_HEX}.md)\n");
    ExportArchive::from_files(BTreeMap::from([
        (format!("Alpha {ALPHA_HEX}.md"), alpha_md.into_bytes()),
        (
            format!("Alpha {ALPHA_HEX}/Beta {BETA_HEX}.md"),
            beta_md.into_bytes(),
        ),
        (format!("Alpha {ALPHA_HEX}/pic.png"), PICTURE.to_vec()),
    ]))
    .unwrap()
}

fn page(id: &str, title: &str, parent: Option<&str>) -> Value {
    let parent = match parent {
        Some(parent) => json!({"type": "page_id", "page_id": parent}),
        None => json!({"type": "workspace", "workspace": true}),
    };
    json!({
        "object": "page",
        "id": id,
        "parent": parent,
        "properties": {"Name": {"id": "title", "type": "title", "title": [
            {"type": "text", "plain_text": title, "text": {"content": title}},
        ]}},
    })
}

fn manifest_for(pages: Vec<Value>, paths: &[&str]) -> Manifest {
    let comments = pages
        .iter()
        .map(|p| (p["id"].as_str().unwrap().to_owned(), false))
        .collect();
    let inventory = Inventory::from_recorded_responses(
        &[list(pages)],
        &BTreeMap::new(),
        vec![],
        comments,
        None,
    )
    .unwrap();
    Manifest::new(
        "workspace-alpha".into(),
        paths.iter().map(|p| (*p).to_owned()).collect(),
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap()
}

fn mint(page_id: &str) -> Result<String> {
    Ok(format!("note-{page_id}"))
}

/// Content with the acquisition-specific block IDs removed.
fn without_block_ids(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            map.remove("block_id");
            for item in map.values_mut() {
                *item = without_block_ids(item.take());
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = without_block_ids(item.take());
            }
        }
        _ => {}
    }
    value
}

#[test]
fn export_and_api_convert_the_same_corpus_to_the_same_content() {
    let mut base = manifest_for(
        vec![page(ALPHA, "Alpha", None), page(BETA, "Beta", Some(ALPHA))],
        &["api", "export"],
    );
    assert_eq!(
        base.acquisition,
        Acquisition {
            api: true,
            export: true,
            degraded_mode: false,
        }
    );
    assert_eq!(base.assign_page_identities(&mut mint).unwrap(), 2);

    let archive = corpus_export();
    let mut via_api = base.clone();
    let mut via_export = base.clone();
    record_export_matches(&mut via_export, &archive);
    base.export_matches = via_export.export_matches.clone();

    // Both files carry their UUID, so both match exactly and nothing
    // needs naming.
    assert_eq!(via_export.export_matches.len(), 2);
    for (path, matched) in &via_export.export_matches {
        assert_eq!(matched.confidence, MatchConfidence::UuidExact, "{path}");
        assert!(!matched.needs_naming(), "{path}");
    }

    let mut export_api = ExportApi::new(&archive, &via_export).unwrap();
    let mut visible = export_api.visible_share_ids();
    visible.sort();
    assert_eq!(visible, [ALPHA.to_owned(), BETA.to_owned()]);

    let mut api = CorpusApi::new();
    for page_id in [ALPHA, BETA] {
        let from_api = convert_page(&mut api, &mut CdnFetcher, &mut via_api, page_id).unwrap();
        let from_export = convert_page(
            &mut export_api,
            &mut ExportFileFetcher::new(&archive),
            &mut via_export,
            page_id,
        )
        .unwrap();

        let api_content = serde_json::to_value(&from_api.content).unwrap();
        let export_content = serde_json::to_value(&from_export.content).unwrap();
        assert_eq!(
            without_block_ids(export_content),
            without_block_ids(api_content),
            "content of {page_id}"
        );
        assert_eq!(from_export.counts, from_api.counts, "counts of {page_id}");
        assert_eq!(from_export.blobs, from_api.blobs, "blobs of {page_id}");
        let refs = |conversion: &id_import_notion::content::PageConversion| {
            conversion
                .references
                .iter()
                .map(|r| (r.surface, r.target_id.clone(), r.target_note_id.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            refs(&from_export),
            refs(&from_api),
            "references of {page_id}"
        );
        assert_eq!(
            from_export.externalised.len(),
            from_api.externalised.len(),
            "externalised of {page_id}"
        );
    }

    // Spot-check the shared shape so the comparison is not vacuous.
    let alpha = convert_page(
        &mut export_api,
        &mut ExportFileFetcher::new(&archive),
        &mut via_export.clone(),
        ALPHA,
    )
    .unwrap();
    let kinds: Vec<&str> = alpha
        .content
        .blocks
        .iter()
        .map(|b| b.block_type.as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "heading_1",
            "paragraph",
            "bulleted_list_item",
            "to_do",
            "numbered_list_item",
            "child_page",
            "image",
            "code",
            "quote",
            "divider",
        ]
    );
    assert_eq!(alpha.content.blocks[2].children.len(), 1);
    assert_eq!(alpha.content.blocks[5].block_id, BETA);
    assert_eq!(
        alpha.content.blocks[6].blob_hash.as_deref(),
        Some(blake3::hash(PICTURE).to_hex().as_str())
    );
    assert!(
        alpha
            .references
            .iter()
            .any(|r| r.surface == ReferenceSurface::ChildPage && r.target_id == BETA)
    );

    // The ledger and acquisition header survive a manifest round trip,
    // and a manifest written before them still loads.
    let saved = serde_json::to_value(&via_export).unwrap();
    let loaded: Manifest = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(loaded.export_matches, via_export.export_matches);
    assert!(loaded.acquisition.export);
    let mut older = saved;
    older.as_object_mut().unwrap().remove("export_matches");
    older.as_object_mut().unwrap().remove("acquisition");
    let older: Manifest = serde_json::from_value(older).unwrap();
    assert!(older.export_matches.is_empty());
    assert_eq!(older.acquisition, Acquisition::default());
}

#[test]
fn every_uncertain_export_match_is_named_in_the_ledger() {
    const GAMMA: &str = "33333333-3333-3333-3333-333333333333";
    const TWIN_A: &str = "55555555-5555-5555-5555-555555555555";
    const TWIN_B: &str = "66666666-6666-6666-6666-666666666666";
    const NESTED: &str = "77777777-7777-7777-7777-777777777777";
    let mut manifest = manifest_for(
        vec![
            page(ALPHA, "Alpha", None),
            page(BETA, "Beta", Some(ALPHA)),
            page(GAMMA, "Gamma", None),
            page(TWIN_A, "Twin", None),
            page(TWIN_B, "Twin", None),
            page(NESTED, "Gamma", Some(ALPHA)),
        ],
        &["api", "export"],
    );
    let ghost = "ffffffffffffffffffffffffffffffff";
    let archive = ExportArchive::from_files(BTreeMap::from([
        (format!("Alpha {ALPHA_HEX}.md"), b"# Alpha\n".to_vec()),
        // Renamed in the export: the API title wins and the conflict is named.
        (
            format!("Alpha {ALPHA_HEX}/Beta {BETA_HEX}.md"),
            b"# Beta renamed\n".to_vec(),
        ),
        // No UUID, one candidate at this path (the nested Gamma is under Alpha).
        ("Gamma.md".to_owned(), b"# Gamma\n".to_vec()),
        // No UUID, two candidates at this path.
        ("Twin.md".to_owned(), b"# Twin\n".to_vec()),
        // No UUID, no candidate.
        ("Orphan.md".to_owned(), b"# Orphan\n".to_vec()),
        // A UUID the API inventory does not have.
        (format!("Ghost {ghost}.md"), b"# Ghost\n".to_vec()),
        // Not a page or database: not an entry.
        ("Gamma/notes.txt".to_owned(), b"ignored".to_vec()),
    ]))
    .unwrap();
    assert_eq!(archive.entries().len(), 6);

    let ledger = match_export(&archive, &manifest.inventory);
    let get = |path: &str| ledger.get(path).unwrap_or_else(|| panic!("{path}"));

    let alpha = get(&format!("Alpha {ALPHA_HEX}.md"));
    assert_eq!(alpha.confidence, MatchConfidence::UuidExact);
    assert_eq!(alpha.inventory_id.as_deref(), Some(ALPHA));
    assert!(!alpha.needs_naming());

    let beta = get(&format!("Alpha {ALPHA_HEX}/Beta {BETA_HEX}.md"));
    assert_eq!(beta.confidence, MatchConfidence::UuidExact);
    assert_eq!(beta.inventory_id.as_deref(), Some(BETA));
    assert_eq!(beta.export_title, "Beta renamed");
    assert_eq!(beta.api_title.as_deref(), Some("Beta"));
    assert!(beta.needs_naming());

    let gamma = get("Gamma.md");
    assert_eq!(gamma.confidence, MatchConfidence::TitlePathUnique);
    assert_eq!(gamma.kind, ExportKind::Page);
    assert!(gamma.needs_naming());

    let twin = get("Twin.md");
    assert_eq!(twin.confidence, MatchConfidence::TitlePathAmbiguous);
    assert_eq!(twin.inventory_id.as_deref(), Some(TWIN_A));
    assert_eq!(twin.candidates, [TWIN_A.to_owned(), TWIN_B.to_owned()]);
    assert!(twin.needs_naming());

    let orphan = get("Orphan.md");
    assert_eq!(orphan.confidence, MatchConfidence::Unmatched);
    assert_eq!(orphan.inventory_id, None);
    assert!(orphan.needs_naming());

    let ghost_match = get(&format!("Ghost {ghost}.md"));
    assert_eq!(ghost_match.confidence, MatchConfidence::Unmatched);
    assert!(ghost_match.needs_naming());

    // Exactly the uncertain entries are named.
    let named: Vec<&String> = ledger
        .iter()
        .filter(|(_, m)| m.needs_naming())
        .map(|(path, _)| path)
        .collect();
    assert_eq!(named.len(), 5);

    record_export_matches(&mut manifest, &archive);
    assert_eq!(manifest.export_matches, ledger);
    assert!(manifest.acquisition.export);
}

#[test]
fn csv_rows_without_page_files_become_entries_and_paths_cannot_escape() {
    let archive = ExportArchive::from_files(BTreeMap::from([
        (
            "Tasks 88888888888888888888888888888888.csv".to_owned(),
            b"Name,Status\nA,open\n".to_vec(),
        ),
        (
            "Tasks 88888888888888888888888888888888_all.csv".to_owned(),
            b"\xef\xbb\xbfName,Status\nA,open\n\"B, with comma\",done\n".to_vec(),
        ),
        (
            "Tasks 88888888888888888888888888888888/A 99999999999999999999999999999999.md"
                .to_owned(),
            b"# A\n\nStatus: open\n\nBody\n".to_vec(),
        ),
    ]))
    .unwrap();
    let entries = archive.entries();
    // One database (the `_all` file wins), its page-backed row, and the
    // row that exists only in the CSV.
    assert_eq!(entries.len(), 3);
    let database = entries
        .iter()
        .find(|e| e.kind == ExportKind::Database)
        .unwrap();
    assert!(database.rel_path.ends_with("_all.csv"));
    assert_eq!(database.title, "Tasks");
    let csv_row = entries.iter().find(|e| e.csv_row.is_some()).unwrap();
    assert_eq!(csv_row.title, "B, with comma");
    assert_eq!(csv_row.ancestors, ["Tasks".to_owned()]);

    let escaped = ExportArchive::from_files(BTreeMap::from([(
        "../outside.md".to_owned(),
        b"# Outside\n".to_vec(),
    )]));
    assert!(escaped.is_err());
}
