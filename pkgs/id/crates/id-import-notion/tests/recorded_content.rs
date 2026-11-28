//! Slice 2 (#13522) acceptance over recorded API responses: converting
//! the fixture pages yields the expected content trees, typed
//! references, and manifest maps (design §8 checks 2 and 6, page/block
//! subset), and re-running identity assignment + conversion over the
//! unchanged corpus changes nothing (§8 check 3, conversion level).

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::content::{
    BlockClass, FileFetcher, ReferenceSurface, SpanLink, convert_page,
};
use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION, NotionApi, Outcome, Stage};
use serde_json::{Value, json};

const ALPHA: &str = "11111111-1111-1111-1111-111111111111";
const BETA: &str = "22222222-2222-2222-2222-222222222222";
const BETA_BLOCK: &str = "44444444-4444-4444-4444-444444444444";
const DELTA_URL: &str = "https://www.notion.so/ws/Delta-33333333333333333333333333333333";
const FILE_URL_MARKER: &str = "files.notion.so";

fn text_span(text: &str, link: Option<&str>) -> Value {
    let mut span = json!({
        "type": "text",
        "plain_text": text,
        "text": {"content": text},
        "annotations": {"bold": false, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default"},
    });
    if let Some(url) = link {
        span["text"]["link"] = json!({"url": url});
        span["href"] = json!(url);
    }
    span
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

struct FixtureApi {
    children: BTreeMap<String, Value>,
}

impl FixtureApi {
    fn new() -> Self {
        let beta_url = "https://www.notion.so/ws/Beta-22222222222222222222222222222222";
        let alpha_blocks = vec![
            block(
                "blk-h1",
                "heading_1",
                json!({"rich_text": [text_span("Alpha Title", None)]}),
                false,
            ),
            block(
                "blk-links",
                "paragraph",
                json!({"rich_text": [
                    text_span("See ", None),
                    text_span("Beta", Some(beta_url)),
                    json!({
                        "type": "mention",
                        "plain_text": "Beta",
                        "href": beta_url,
                        "mention": {"type": "page", "page": {"id": BETA}},
                        "annotations": {"bold": false, "italic": false, "strikethrough": false, "underline": false, "code": false, "color": "default"},
                    }),
                    text_span("Delta", Some(DELTA_URL)),
                    text_span("Example", Some("https://example.com/x")),
                    text_span(
                        "Beta section",
                        Some("https://www.notion.so/ws/Beta-22222222222222222222222222222222#44444444444444444444444444444444"),
                    ),
                ]}),
                false,
            ),
            block(
                "blk-list",
                "bulleted_list_item",
                json!({"rich_text": [text_span("Item", None)]}),
                true,
            ),
            block(
                "blk-todo",
                "to_do",
                json!({"rich_text": [text_span("Do it", None)], "checked": true}),
                false,
            ),
            block(BETA, "child_page", json!({"title": "Beta"}), true),
            block(
                "blk-img",
                "image",
                json!({
                    "type": "file",
                    "file": {"url": "https://files.notion.so/img?sig=fixture-secret", "expiry_time": "2026-11-24T00:00:00Z"},
                    "caption": [text_span("Pic", None)],
                }),
                false,
            ),
            block(
                "blk-file",
                "file",
                json!({
                    "type": "file",
                    "file": {"url": "https://files.notion.so/doc?sig=fixture-secret", "expiry_time": "2026-11-24T00:00:00Z"},
                    "caption": [],
                }),
                false,
            ),
            block(
                "blk-pdf",
                "pdf",
                json!({
                    "type": "file",
                    "file": {"url": "https://files.notion.so/bad?sig=fixture-secret", "expiry_time": "2026-11-24T00:00:00Z"},
                    "caption": [],
                }),
                false,
            ),
            block(
                "blk-toggle",
                "toggle",
                json!({"rich_text": [text_span("More", None)], "color": "blue_background"}),
                true,
            ),
            block(
                "blk-future",
                "future_widget",
                json!({"future_widget": {"level": 3}}),
                false,
            ),
            block("blk-div", "divider", json!({}), false),
            block(
                "blk-code",
                "code",
                json!({"rich_text": [text_span("fn main() {}", None)], "language": "rust"}),
                false,
            ),
            block(
                "blk-quote",
                "quote",
                json!({"rich_text": [text_span("Quoted", None)]}),
                false,
            ),
            block(
                "blk-bookmark",
                "bookmark",
                json!({"url": "https://example.com/article", "caption": []}),
                false,
            ),
        ];
        let children = BTreeMap::from([
            (ALPHA.to_owned(), list(alpha_blocks)),
            (
                "blk-list".to_owned(),
                list(vec![block(
                    "blk-nested",
                    "paragraph",
                    json!({"rich_text": [text_span("Nested", None)]}),
                    false,
                )]),
            ),
            (
                "blk-toggle".to_owned(),
                list(vec![block(
                    "blk-toggle-child",
                    "paragraph",
                    json!({"rich_text": [text_span("Inside", None)]}),
                    false,
                )]),
            ),
            (
                BETA.to_owned(),
                list(vec![block(
                    BETA_BLOCK,
                    "paragraph",
                    json!({"rich_text": [
                        text_span("Back to ", None),
                        text_span("Alpha", Some("https://www.notion.so/ws/Alpha-11111111111111111111111111111111")),
                    ]}),
                    false,
                )]),
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
        bail!("unused in content fixtures")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused in content fixtures")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        Ok(true)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![ALPHA.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        None
    }
}

struct FixtureFetcher {
    fetched: Vec<String>,
}

impl FileFetcher for FixtureFetcher {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        self.fetched.push(url.to_owned());
        if url.contains("bad") {
            bail!("404 from fixture CDN");
        }
        Ok(b"fixture-bytes".to_vec())
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
        page(ALPHA, "Alpha"),
        page(BETA, "Beta"),
        json!({
            "object": "block",
            "id": BETA_BLOCK,
            "parent": {"type": "page_id", "page_id": BETA},
            "type": "paragraph",
            "paragraph": {"rich_text": []},
        }),
    ]);
    let inventory = Inventory::from_recorded_responses(
        &[search],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([(ALPHA.to_owned(), true), (BETA.to_owned(), true)]),
        None,
    )
    .unwrap();
    Manifest::new(
        "workspace-alpha".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap()
}

fn mint(page_id: &str) -> Result<String> {
    Ok(format!("note-{page_id}"))
}

#[test]
fn recorded_pages_convert_to_the_expected_content() {
    let mut manifest = fixture_manifest();
    let mut api = FixtureApi::new();
    let mut fetcher = FixtureFetcher { fetched: vec![] };

    // Conversion before the identity pass refuses, per the §3 two-pass rule.
    let refused = convert_page(&mut api, &mut fetcher, &mut manifest, BETA);
    assert!(refused.is_err());

    assert_eq!(manifest.assign_page_identities(&mut mint).unwrap(), 2);
    assert_eq!(manifest.assign_page_identities(&mut mint).unwrap(), 0);

    let conversion = convert_page(&mut api, &mut fetcher, &mut manifest, ALPHA).unwrap();
    let content = &conversion.content;
    assert_eq!(content.note_id, format!("note-{ALPHA}"));
    assert_eq!(content.title, "Alpha");
    let kinds: Vec<&str> = content
        .blocks
        .iter()
        .map(|block| block.block_type.as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "heading_1",
            "paragraph",
            "bulleted_list_item",
            "to_do",
            "child_page",
            "image",
            "file",
            "pdf",
            "toggle",
            "future_widget",
            "divider",
            "code",
            "quote",
            "bookmark",
        ]
    );
    assert_eq!(conversion.counts.clean, 13);
    assert_eq!(conversion.counts.degraded, 1);
    assert_eq!(conversion.counts.unsupported, 1);

    // Nested list children stay inside their parent block.
    let list_block = &content.blocks[2];
    assert_eq!(list_block.children.len(), 1);
    assert_eq!(list_block.children[0].block_id, "blk-nested");
    // To-do state and code language survive.
    assert_eq!(content.blocks[3].checked, Some(true));
    assert_eq!(content.blocks[11].language.as_deref(), Some("rust"));
    // The child page is a reference, not inlined content.
    let child_page = &content.blocks[4];
    assert_eq!(child_page.title.as_deref(), Some("Beta"));
    assert!(child_page.children.is_empty());

    // §3 rewriting: internal spans became note references, the outside
    // link stayed verbatim and was counted externalised.
    let spans = &content.blocks[1].rich_text;
    assert_eq!(spans.len(), 6);
    assert_eq!(spans[0].link, None);
    assert_eq!(
        spans[1].link,
        Some(SpanLink::Note {
            note_id: format!("note-{BETA}"),
            block_id: None,
        })
    );
    assert_eq!(
        spans[2].link,
        Some(SpanLink::Note {
            note_id: format!("note-{BETA}"),
            block_id: None,
        })
    );
    assert_eq!(
        spans[3].link,
        Some(SpanLink::External {
            url: DELTA_URL.to_owned(),
        })
    );
    assert_eq!(
        spans[4].link,
        Some(SpanLink::External {
            url: "https://example.com/x".to_owned(),
        })
    );
    assert_eq!(
        spans[5].link,
        Some(SpanLink::Note {
            note_id: format!("note-{BETA}"),
            block_id: Some(BETA_BLOCK.to_owned()),
        })
    );
    assert_eq!(conversion.externalised.len(), 1);
    assert_eq!(
        conversion.externalised[0].target_id,
        "33333333333333333333333333333333"
    );
    assert_eq!(
        conversion.externalised[0].surface,
        ReferenceSurface::InlineLink
    );

    // References in document order: inline link, mention, block link,
    // then the child page.
    let surfaces: Vec<ReferenceSurface> = conversion
        .references
        .iter()
        .map(|reference| reference.surface)
        .collect();
    assert_eq!(
        surfaces,
        [
            ReferenceSurface::InlineLink,
            ReferenceSurface::Mention,
            ReferenceSurface::InlineLink,
            ReferenceSurface::ChildPage,
        ]
    );
    assert_eq!(
        conversion.references[2].target_block_id.as_deref(),
        Some(BETA_BLOCK)
    );
    assert_eq!(
        conversion.references[3].target_note_id.as_deref(),
        Some(format!("note-{BETA}").as_str())
    );

    // §4: two file blocks with identical bytes share one blob; the
    // unfetchable PDF is a named missing file, not an aborted import.
    let expected_hash = blake3::hash(b"fixture-bytes").to_hex().as_str().to_owned();
    assert_eq!(conversion.blobs.len(), 1);
    assert!(conversion.blobs.contains_key(&expected_hash));
    assert_eq!(
        content.blocks[5].blob_hash.as_deref(),
        Some(expected_hash.as_str())
    );
    assert_eq!(
        content.blocks[6].blob_hash.as_deref(),
        Some(expected_hash.as_str())
    );
    assert_eq!(conversion.missing_files.len(), 1);
    assert_eq!(conversion.missing_files[0].block_id, "blk-pdf");
    assert!(content.blocks[7].missing_file.is_some());
    assert_eq!(fetcher.fetched.len(), 3);

    // Degraded and unsupported blocks are preserved verbatim, children
    // embedded, file URLs stripped.
    let toggle = &content.blocks[8];
    assert_eq!(toggle.class, BlockClass::Degraded);
    let verbatim = toggle.verbatim.as_ref().unwrap();
    assert_eq!(verbatim["toggle"]["color"], "blue_background");
    assert_eq!(verbatim["children"][0]["id"], "blk-toggle-child");
    let future = &content.blocks[9];
    assert_eq!(future.class, BlockClass::Unsupported);
    assert!(future.verbatim.is_some());

    // Manifest state: identities, the file map, staged progress.
    assert_eq!(
        manifest.block_id_to_identity["blk-h1"].0,
        format!("note-{ALPHA}")
    );
    assert_eq!(manifest.block_id_to_identity["blk-h1"].1, "blk-h1");
    assert_eq!(
        manifest.block_id_to_identity[BETA].0,
        format!("note-{ALPHA}")
    );
    assert_eq!(manifest.block_id_to_identity[BETA].1, BETA);
    assert_eq!(manifest.file_blobs.len(), 2);
    assert_eq!(manifest.file_blobs["blk-img"].blob_hash, expected_hash);
    assert_eq!(manifest.file_blobs["blk-file"].blob_hash, expected_hash);
    assert_eq!(manifest.file_blobs["blk-img"].size_bytes, 13);
    let progress = &manifest.objects[ALPHA];
    assert_eq!(progress.stage, Stage::References);
    assert_eq!(progress.outcome, Outcome::Degraded);

    // §6.5: no expiring file URL or fixture secret is persisted in the
    // manifest or the content form.
    let manifest_json = serde_json::to_string(&manifest).unwrap();
    let content_json = serde_json::to_string(&conversion.content).unwrap();
    for artifact in [&manifest_json, &content_json] {
        assert!(!artifact.contains(FILE_URL_MARKER), "{artifact}");
        assert!(!artifact.contains("fixture-secret"), "{artifact}");
    }

    // Beta converts clean and links back to Alpha (the backlink pair).
    let beta = convert_page(&mut api, &mut fetcher, &mut manifest, BETA).unwrap();
    assert_eq!(beta.counts.clean, 1);
    assert_eq!(beta.references.len(), 1);
    assert_eq!(
        beta.references[0].target_note_id.as_deref(),
        Some(format!("note-{ALPHA}").as_str())
    );
    assert_eq!(manifest.objects[BETA].outcome, Outcome::Clean);

    // §8 check 3 at conversion level: re-running over the unchanged
    // corpus assigns nothing and reproduces identical artifacts.
    let maps_before = (
        manifest.page_id_to_note_id.clone(),
        manifest.block_id_to_identity.clone(),
        manifest.file_blobs.clone(),
    );
    assert_eq!(manifest.assign_page_identities(&mut mint).unwrap(), 0);
    let rerun = convert_page(&mut api, &mut fetcher, &mut manifest, ALPHA).unwrap();
    assert_eq!(rerun.content, conversion.content);
    assert_eq!(rerun.references, conversion.references);
    assert_eq!(
        maps_before,
        (
            manifest.page_id_to_note_id.clone(),
            manifest.block_id_to_identity.clone(),
            manifest.file_blobs.clone(),
        )
    );
}

#[test]
fn converting_an_unknown_or_non_page_object_fails() {
    let mut manifest = fixture_manifest();
    let mut api = FixtureApi::new();
    let mut fetcher = FixtureFetcher { fetched: vec![] };
    assert!(convert_page(&mut api, &mut fetcher, &mut manifest, "page-missing").is_err());
    assert!(convert_page(&mut api, &mut fetcher, &mut manifest, BETA_BLOCK).is_err());
}
