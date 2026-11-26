//! Slice 4 sub-slice 3 (#13524, design record §2.3): a synced block's
//! content converts once, at the source block's page; every duplicate
//! becomes a typed reference to (source note, source block) in the
//! reference pass; a duplicate whose source is outside the import set
//! keeps its verbatim blob and is counted externalised.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::content::{
    ContentBlock, FileFetcher, ReferenceSurface, SpanLink, SyncedResolution, convert_page,
    resolve_synced_references,
};
use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION, NotionApi};
use serde_json::{Value, json};

const SOURCE_PAGE: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";
const REF_PAGE: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
const SOURCE_BLOCK: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
const OUTSIDE_BLOCK: &str = "ffffffff-ffff-ffff-ffff-ffffffffffff";

fn text(content: &str) -> Value {
    json!([{"type": "text", "plain_text": content, "text": {"content": content}}])
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

fn duplicate(id: &str, source: &str) -> Value {
    // Notion reports `has_children` on a duplicate (its children are the
    // source's); the fixture serves no children for it, so a fetch would
    // fail the test.
    block(
        id,
        "synced_block",
        json!({"synced_from": {"type": "block_id", "block_id": source}}),
        true,
    )
}

struct FixtureApi {
    children: BTreeMap<String, Value>,
}

impl FixtureApi {
    fn new() -> Self {
        let children = BTreeMap::from([
            (
                SOURCE_PAGE.to_owned(),
                list(vec![
                    block(
                        SOURCE_BLOCK,
                        "synced_block",
                        json!({"synced_from": null}),
                        true,
                    ),
                    duplicate("dup-same-page", SOURCE_BLOCK),
                ]),
            ),
            (
                SOURCE_BLOCK.to_owned(),
                list(vec![block(
                    "shared-p",
                    "paragraph",
                    json!({"rich_text": text("Shared")}),
                    false,
                )]),
            ),
            (
                REF_PAGE.to_owned(),
                list(vec![
                    duplicate("dup-other-page", SOURCE_BLOCK),
                    duplicate("dup-outside", OUTSIDE_BLOCK),
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
        bail!("unused in synced fixtures")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused in synced fixtures")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        Ok(true)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![SOURCE_PAGE.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        None
    }
}

struct NoFiles;

impl FileFetcher for NoFiles {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        bail!("synced fixtures fetch no files, asked for {url}")
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
    let search = list(vec![
        page(SOURCE_PAGE, "Source"),
        page(REF_PAGE, "Referrer"),
        json!({
            "object": "block",
            "id": SOURCE_BLOCK,
            "parent": {"type": "page_id", "page_id": SOURCE_PAGE},
            "type": "synced_block",
            "synced_block": {"synced_from": null},
        }),
    ]);
    let inventory = Inventory::from_recorded_responses(
        &[search],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([(SOURCE_PAGE.to_owned(), true), (REF_PAGE.to_owned(), true)]),
        None,
    )
    .unwrap();
    Manifest::new(
        "workspace-synced".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap()
}

fn mint(page_id: &str) -> Result<String> {
    Ok(format!("note-{page_id}"))
}

fn find<'a>(blocks: &'a [ContentBlock], id: &str) -> &'a ContentBlock {
    blocks
        .iter()
        .find(|block| block.block_id == id)
        .unwrap_or_else(|| panic!("block {id} missing"))
}

#[test]
fn synced_blocks_convert_once_and_duplicates_reference_the_source_13524() {
    let mut manifest = fixture_manifest();
    manifest.assign_page_identities(&mut mint).unwrap();
    let mut api = FixtureApi::new();

    // The referring page converts first: its duplicates wait.
    let referrer = convert_page(&mut api, &mut NoFiles, &mut manifest, REF_PAGE).unwrap();
    assert_eq!(referrer.pending_synced.len(), 2);
    let mut early = vec![referrer.clone()];
    // The source belongs to an imported page that has not converted, so
    // the pass refuses instead of calling it external.
    assert!(resolve_synced_references(&mut manifest, &mut early).is_err());

    let source = convert_page(&mut api, &mut NoFiles, &mut manifest, SOURCE_PAGE).unwrap();
    // The source converts its content here, once.
    let source_block = find(&source.content.blocks, SOURCE_BLOCK);
    assert!(source_block.synced_from.is_none());
    assert!(source_block.verbatim.is_none());
    assert_eq!(source_block.children[0].rich_text[0].text, "Shared");
    assert_eq!(
        manifest.block_id_to_identity["shared-p"].0,
        format!("note-{SOURCE_PAGE}")
    );

    let mut conversions = vec![source, referrer];
    let resolution = resolve_synced_references(&mut manifest, &mut conversions).unwrap();
    assert_eq!(
        resolution,
        SyncedResolution {
            resolved: 2,
            externalised: 1
        }
    );

    let expected_link = Some(SpanLink::Note {
        note_id: format!("note-{SOURCE_PAGE}"),
        block_id: Some(SOURCE_BLOCK.to_owned()),
    });
    for (conversion, duplicate_id) in [
        (&conversions[0], "dup-same-page"),
        (&conversions[1], "dup-other-page"),
    ] {
        let duplicate = find(&conversion.content.blocks, duplicate_id);
        assert_eq!(duplicate.synced_from.as_deref(), Some(SOURCE_BLOCK));
        assert_eq!(duplicate.link, expected_link);
        // No copy of the content: the duplicate has no children.
        assert!(duplicate.children.is_empty());
        assert!(duplicate.verbatim.is_none());
        assert!(conversion.references.iter().any(|reference| {
            reference.from_block_id == duplicate_id
                && reference.target_block_id.as_deref() == Some(SOURCE_BLOCK)
                && reference.surface == ReferenceSurface::SyncedBlock
        }));
    }

    // The out-of-set source: verbatim blob plus an externalised count.
    let referrer = &conversions[1];
    let outside = find(&referrer.content.blocks, "dup-outside");
    assert!(outside.link.is_none());
    assert_eq!(
        outside.verbatim.as_ref().unwrap()["synced_block"]["synced_from"]["block_id"],
        OUTSIDE_BLOCK
    );
    assert!(referrer.externalised.iter().any(|item| {
        item.from_block_id == "dup-outside" && item.surface == ReferenceSurface::SyncedBlock
    }));
    assert_eq!(referrer.losses.synced_externalised, 1);
    assert_eq!(manifest.page_losses[REF_PAGE].synced_externalised, 1);
    assert_eq!(manifest.page_losses[SOURCE_PAGE].synced_externalised, 0);

    // The pass consumes its pending entries: a second run changes nothing.
    let before = conversions.clone();
    let again = resolve_synced_references(&mut manifest, &mut conversions).unwrap();
    assert_eq!(again, SyncedResolution::default());
    assert_eq!(conversions, before);
}
