//! Slice 4 sub-slice 2 (#13524, design record §2.3–2.4) acceptance over
//! recorded API responses: every §2.5 degraded class in the corpus
//! converts structurally, each dropped presentation attribute is counted
//! by name, and the loss ledger's class totals equal the corpus's known
//! counts (design #6436 §8 check 4). A payload a converter cannot parse
//! keeps the slice-2 verbatim blob, counted as a fallback, never an
//! error that aborts the page.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::content::{
    BlockClass, FileFetcher, ReferenceSurface, SpanLink, convert_page, resolve_synced_references,
};
use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION, NotionApi, Outcome};
use serde_json::{Value, json};

const PAGE: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
const OTHER_URL: &str = "https://www.notion.so/ws/Other-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ICON_FILE_URL: &str = "https://files.notion.so/icon.png?sig=fixture-secret";

fn text(content: &str) -> Value {
    json!([{
        "type": "text",
        "plain_text": content,
        "text": {"content": content},
        "annotations": {"bold": false, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default"},
    }])
}

fn block(id: &str, block_type: &str, payload: Value, has_children: bool) -> Value {
    let mut value =
        json!({"object": "block", "id": id, "type": block_type, "has_children": has_children});
    value[block_type] = payload;
    value
}

fn list(results: Vec<Value>) -> Value {
    json!({"object": "list", "results": results, "has_more": false})
}

struct FixtureApi {
    children: BTreeMap<String, Value>,
}

impl FixtureApi {
    fn new() -> Self {
        let top = vec![
            block(
                "deg-toggle",
                "toggle",
                json!({"rich_text": text("Details"), "color": "red"}),
                true,
            ),
            block(
                "deg-callout",
                "callout",
                json!({"rich_text": text("Heads up"), "icon": {"type": "emoji", "emoji": "💡"}, "color": "gray_background"}),
                false,
            ),
            block(
                "deg-callout-file",
                "callout",
                json!({"rich_text": text("Logo"), "icon": {"type": "file", "file": {"url": ICON_FILE_URL}}, "color": "default"}),
                false,
            ),
            block("deg-cols", "column_list", json!({}), true),
            block(
                "deg-table",
                "table",
                json!({"table_width": 2, "has_column_header": true, "has_row_header": false}),
                true,
            ),
            block(
                "deg-eq",
                "equation",
                json!({"expression": "e^{i\\pi} + 1 = 0"}),
                false,
            ),
            block(
                "deg-embed",
                "embed",
                json!({"url": "https://example.com/widget", "caption": text("Widget")}),
                false,
            ),
            block(
                "deg-preview",
                "link_preview",
                json!({"url": OTHER_URL}),
                false,
            ),
            block(
                "deg-synced",
                "synced_block",
                json!({"synced_from": {"type": "block_id", "block_id": "elsewhere"}}),
                false,
            ),
            block(
                "deg-toc",
                "table_of_contents",
                json!({"color": "default"}),
                false,
            ),
            // Malformed: an equation with no expression string.
            block("deg-eq-bad", "equation", json!({"expression": 42}), false),
        ];
        let children = BTreeMap::from([
            (PAGE.to_owned(), list(top)),
            (
                "deg-toggle".to_owned(),
                list(vec![block(
                    "deg-toggle-child",
                    "paragraph",
                    json!({"rich_text": text("Inside")}),
                    false,
                )]),
            ),
            (
                "deg-cols".to_owned(),
                list(vec![
                    block("deg-col-a", "column", json!({"width_ratio": 0.25}), true),
                    block("deg-col-b", "column", json!({}), true),
                ]),
            ),
            (
                "deg-col-a".to_owned(),
                list(vec![block(
                    "deg-col-a-p",
                    "paragraph",
                    json!({"rich_text": text("Left")}),
                    false,
                )]),
            ),
            (
                "deg-col-b".to_owned(),
                list(vec![block(
                    "deg-col-b-p",
                    "paragraph",
                    json!({"rich_text": text("Right")}),
                    false,
                )]),
            ),
            (
                "deg-table".to_owned(),
                list(vec![
                    block(
                        "deg-row-1",
                        "table_row",
                        json!({"cells": [text("Name"), text("Count")]}),
                        false,
                    ),
                    block(
                        "deg-row-2",
                        "table_row",
                        json!({"cells": [text("Apples"), text("3")]}),
                        false,
                    ),
                ]),
            ),
        ]);
        Self { children }
    }
}

impl NotionApi for FixtureApi {
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
        bail!("unused in degradation fixtures")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused in degradation fixtures")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        Ok(true)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![PAGE.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        None
    }
}

struct NoFiles;

impl FileFetcher for NoFiles {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        bail!("degradation fixtures fetch no files, asked for {url}")
    }
}

fn fixture_manifest() -> Manifest {
    let page = |id: &str, title: &str| {
        json!({
            "object": "page",
            "id": id,
            "parent": {"type": "workspace", "workspace": true},
            "properties": {"Name": {"id": "title", "type": "title", "title": [
                {"type": "text", "plain_text": title, "text": {"content": title}},
            ]}},
        })
    };
    let inventory = Inventory::from_recorded_responses(
        &[list(vec![page(PAGE, "Degraded"), page(OTHER, "Other")])],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([(PAGE.to_owned(), true), (OTHER.to_owned(), true)]),
        None,
    )
    .unwrap();
    Manifest::new(
        "workspace-degraded".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap()
}

fn mint(page_id: &str) -> Result<String> {
    Ok(format!("note-{page_id}"))
}

fn find<'a>(
    blocks: &'a [id_import_notion::content::ContentBlock],
    id: &str,
) -> &'a id_import_notion::content::ContentBlock {
    blocks
        .iter()
        .find(|block| block.block_id == id)
        .unwrap_or_else(|| panic!("block {id} missing"))
}

#[test]
fn degraded_classes_convert_structurally_with_counted_losses_13524() {
    let mut manifest = fixture_manifest();
    manifest.assign_page_identities(&mut mint).unwrap();
    let mut api = FixtureApi::new();
    let mut conversion = convert_page(&mut api, &mut NoFiles, &mut manifest, PAGE).unwrap();
    let blocks = &conversion.content.blocks;

    // Toggle: text and children convert; the colour is the named loss.
    let toggle = find(blocks, "deg-toggle");
    assert_eq!(toggle.class, BlockClass::Degraded);
    assert!(toggle.verbatim.is_none());
    assert_eq!(toggle.rich_text[0].text, "Details");
    assert_eq!(toggle.children[0].block_id, "deg-toggle-child");
    assert_eq!(toggle.children[0].class, BlockClass::Clean);

    // Callout: the emoji icon stays as provenance; a hosted icon file is
    // a counted loss and its expiring URL is never kept.
    let callout = find(blocks, "deg-callout");
    assert_eq!(callout.icon.as_deref(), Some("💡"));
    assert_eq!(callout.rich_text[0].text, "Heads up");
    let file_icon = find(blocks, "deg-callout-file");
    assert!(file_icon.icon.is_none());
    assert!(file_icon.verbatim.is_none());

    // Columns keep order and nesting; the width ratio is the loss.
    let cols = find(blocks, "deg-cols");
    let column_ids: Vec<&str> = cols.children.iter().map(|c| c.block_id.as_str()).collect();
    assert_eq!(column_ids, ["deg-col-a", "deg-col-b"]);
    assert_eq!(cols.children[0].children[0].rich_text[0].text, "Left");
    assert_eq!(cols.children[1].children[0].rich_text[0].text, "Right");

    // Simple table keeps its width, rows and cells.
    let table = find(blocks, "deg-table");
    assert_eq!(table.columns, Some(2));
    let rows: Vec<Vec<&str>> = table
        .children
        .iter()
        .map(|row| row.cells.iter().map(|cell| cell[0].text.as_str()).collect())
        .collect();
    assert_eq!(rows, [["Name", "Count"], ["Apples", "3"]]);

    // Equation keeps its source text.
    assert_eq!(
        find(blocks, "deg-eq").expression.as_deref(),
        Some("e^{i\\pi} + 1 = 0")
    );

    // Embed: URL plus an external link reference; the caption survives.
    let embed = find(blocks, "deg-embed");
    assert_eq!(embed.url.as_deref(), Some("https://example.com/widget"));
    assert_eq!(
        embed.link,
        Some(SpanLink::External {
            url: "https://example.com/widget".into()
        })
    );
    assert_eq!(embed.rich_text[0].text, "Widget");

    // A link preview of an imported page becomes a typed reference.
    let preview = find(blocks, "deg-preview");
    assert_eq!(
        preview.link,
        Some(SpanLink::Note {
            note_id: format!("note-{OTHER}"),
            block_id: None,
        })
    );
    assert!(conversion.references.iter().any(|reference| {
        reference.from_block_id == "deg-preview"
            && reference.target_id == OTHER
            && reference.surface == ReferenceSurface::InlineLink
    }));

    // A synced duplicate names its source and waits on the reference pass.
    let synced = find(blocks, "deg-synced");
    assert_eq!(synced.synced_from.as_deref(), Some("elsewhere"));
    assert!(synced.verbatim.is_none());
    assert_eq!(conversion.pending_synced.len(), 1);
    // Unsupported stays verbatim.
    let toc = find(blocks, "deg-toc");
    assert_eq!(toc.class, BlockClass::Unsupported);
    assert!(toc.verbatim.is_some());
    // A payload the converter cannot parse keeps the slice-2 verbatim blob.
    let bad = find(blocks, "deg-eq-bad");
    assert_eq!(bad.class, BlockClass::Degraded);
    assert!(bad.expression.is_none());
    assert_eq!(bad.verbatim.as_ref().unwrap()["equation"]["expression"], 42);

    // §8 check 4: the ledger's class totals equal the corpus's known
    // counts, and reconcile to the per-class block counts.
    let losses = &conversion.losses;
    let expected_classes = BTreeMap::from([
        ("callout".to_owned(), 2),
        ("column".to_owned(), 2),
        ("column_list".to_owned(), 1),
        ("embed".to_owned(), 1),
        ("equation".to_owned(), 2),
        ("link_preview".to_owned(), 1),
        ("synced_block".to_owned(), 1),
        ("table".to_owned(), 1),
        ("table_row".to_owned(), 2),
        ("toggle".to_owned(), 1),
    ]);
    assert_eq!(losses.per_class, expected_classes);
    assert_eq!(
        losses.unsupported,
        BTreeMap::from([("table_of_contents".to_owned(), 1)])
    );
    let expected_attributes = BTreeMap::from([
        ("callout.color".to_owned(), 1),
        ("callout.icon_file".to_owned(), 1),
        ("column.width_ratio".to_owned(), 1),
        ("embed.preview".to_owned(), 1),
        ("link_preview.preview".to_owned(), 1),
        ("table.has_column_header".to_owned(), 1),
        ("toggle.color".to_owned(), 1),
    ]);
    assert_eq!(losses.per_attribute, expected_attributes);
    assert_eq!(
        losses.verbatim_fallback,
        BTreeMap::from([("equation".to_owned(), 1)])
    );
    assert_eq!(conversion.counts.degraded, 14);
    assert_eq!(conversion.counts.unsupported, 1);
    assert_eq!(
        losses.class_total(),
        conversion.counts.degraded + conversion.counts.unsupported
    );
    assert_eq!(manifest.page_losses[PAGE], *losses);
    assert_eq!(manifest.objects[PAGE].outcome, Outcome::Degraded);

    // §6.5: the hosted icon's expiring URL is persisted nowhere.
    let manifest_json = serde_json::to_string(&manifest).unwrap();
    let content_json = serde_json::to_string(&conversion.content).unwrap();
    for artifact in [&manifest_json, &content_json] {
        assert!(!artifact.contains("fixture-secret"), "{artifact}");
    }

    // Re-running over the unchanged corpus reproduces the same artifacts.
    let rerun = convert_page(&mut api, &mut NoFiles, &mut manifest, PAGE).unwrap();
    assert_eq!(rerun.content, conversion.content);
    assert_eq!(rerun.losses, conversion.losses);

    // The reference pass finds the synced source outside the import set:
    // the duplicate keeps its verbatim blob and is counted externalised.
    let resolution =
        resolve_synced_references(&mut manifest, std::slice::from_mut(&mut conversion)).unwrap();
    assert_eq!(resolution.externalised, 1);
    assert!(
        find(&conversion.content.blocks, "deg-synced")
            .verbatim
            .is_some()
    );
    assert_eq!(conversion.losses.synced_externalised, 1);
    assert_eq!(manifest.page_losses[PAGE].synced_externalised, 1);
}

#[test]
fn pre_slice4_manifests_load_with_an_empty_loss_ledger_13524() {
    let manifest = fixture_manifest();
    let mut value = serde_json::to_value(&manifest).unwrap();
    value.as_object_mut().unwrap().remove("page_losses");
    let loaded: Manifest = serde_json::from_value(value).unwrap();
    assert!(loaded.page_losses.is_empty());
}
