//! Slice 5 (#13525, design #6436 §6.4) acceptance over recorded API
//! responses: the persisted fidelity report accounts for every
//! inventory object exactly once (§8 check 1), names every non-clean
//! item with its reason, carries the §2.5 degradation class counts,
//! externalised links, missing files and comments coverage, and refuses
//! to generate when the manifest does not reconcile to the inventory.
//! The oracle is the fixture itself (known counts) plus the
//! reconciliation invariant: clean + degraded + failed + skipped must
//! equal the inventory total.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::content::{FileFetcher, convert_page};
use id_import_notion::fidelity::generate_report;
use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION, NotionApi};
use serde_json::{Value, json};

const CLEAN: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
const MESSY: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
const ELSEWHERE_URL: &str = "https://www.notion.so/ws/Elsewhere-cccccccccccccccccccccccccccccccc";
const BAD_FILE_URL: &str = "https://files.notion.so/gone?sig=fixture-secret";

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
        let clean = vec![
            block(
                "clean-h1",
                "heading_1",
                json!({"rich_text": [text_span("Clean Title", None)]}),
                false,
            ),
            block(
                "clean-p",
                "paragraph",
                json!({"rich_text": [text_span("All good here.", None)]}),
                false,
            ),
        ];
        let messy = vec![
            block(
                "messy-toggle",
                "toggle",
                json!({"rich_text": [text_span("Details", None)], "color": "red"}),
                true,
            ),
            block(
                "messy-link",
                "paragraph",
                json!({"rich_text": [
                    text_span("See ", None),
                    text_span("elsewhere", Some(ELSEWHERE_URL)),
                ]}),
                false,
            ),
            block(
                "messy-img",
                "image",
                json!({
                    "type": "file",
                    "file": {"url": BAD_FILE_URL, "expiry_time": "2026-11-24T00:00:00Z"},
                    "caption": [],
                }),
                false,
            ),
        ];
        let toggle_kid = vec![block(
            "messy-toggle-child",
            "paragraph",
            json!({"rich_text": [text_span("Inside", None)]}),
            false,
        )];
        Self {
            children: BTreeMap::from([
                (CLEAN.to_owned(), list(clean)),
                (MESSY.to_owned(), list(messy)),
                ("messy-toggle".to_owned(), list(toggle_kid)),
            ]),
        }
    }
}

fn comment(id: &str, body: &str) -> Value {
    json!({
        "object": "comment",
        "id": id,
        "discussion_id": "discussion-1",
        "created_time": "2026-11-20T00:00:00Z",
        "created_by": {"object": "user", "id": "user-1", "name": "Reviewer"},
        "parent": {"type": "page_id", "page_id": CLEAN},
        "rich_text": [text_span(body, None)],
    })
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
        bail!("unused in fidelity fixtures")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused in fidelity fixtures")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
        Ok(true)
    }
    fn comments(&mut self, page_id: &str, _cursor: Option<&str>) -> Result<Value> {
        if page_id == CLEAN {
            return Ok(list(vec![comment("comment-1", "Looks right.")]));
        }
        bail!("comments endpoint must not be called for {page_id}")
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![CLEAN.to_owned(), MESSY.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        Some("business".into())
    }
}

struct NoFiles;

impl FileFetcher for NoFiles {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>> {
        bail!("fidelity fixtures fetch no files, asked for {url}")
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
        &[list(vec![page(CLEAN, "Clean"), page(MESSY, "Messy")])],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([(CLEAN.to_owned(), true), (MESSY.to_owned(), false)]),
        Some("business".into()),
    )
    .unwrap();
    Manifest::new(
        "workspace-fidelity".into(),
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
fn fidelity_report_reconciles_and_names_everything_13525() {
    let mut manifest = fixture_manifest();
    manifest.assign_page_identities(&mut mint).unwrap();
    let mut api = FixtureApi::new();
    convert_page(&mut api, &mut NoFiles, &mut manifest, CLEAN).unwrap();
    convert_page(&mut api, &mut NoFiles, &mut manifest, MESSY).unwrap();
    id_import_notion::comments::convert_comments(&mut api, &mut manifest, CLEAN).unwrap();
    id_import_notion::comments::convert_comments(&mut api, &mut manifest, MESSY).unwrap();

    let report = generate_report(&manifest).unwrap();

    // §8 check 1: every inventory object accounted for exactly once.
    assert!(report.reconciles());
    assert_eq!(report.total_objects, 2);
    assert_eq!(report.clean, 1);
    assert_eq!(report.degraded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.skipped, 0);

    // Every non-clean item carries its reason.
    assert_eq!(report.non_clean.len(), 1);
    assert_eq!(report.non_clean[0].object_id, MESSY);
    assert!(report.non_clean[0].reason.as_deref().is_some());

    // §8 check 4 (fixture subset): the toggle degradation is counted.
    assert_eq!(report.degradation_classes.get("toggle"), Some(&1));

    // §3: the link outside the import set is named, not dropped.
    assert_eq!(report.externalised_links.len(), 1);
    assert_eq!(report.externalised_links[0].page_id, MESSY);
    assert_eq!(report.externalised_links[0].from_block_id, "messy-link");

    // §4: the unfetchable file is named with its page and reason, and
    // the expiring URL never appears in the reason.
    assert_eq!(report.missing_files.len(), 1);
    assert_eq!(report.missing_files[0].page_id, MESSY);
    assert_eq!(report.missing_files[0].block_id, "messy-img");
    assert!(!report.missing_files[0].reason.contains("files.notion.so"));

    // §2.4: coverage names the covered page and the unavailable one.
    assert_eq!(report.comments_covered, vec![CLEAN.to_owned()]);
    assert_eq!(report.comments_unavailable, vec![MESSY.to_owned()]);
    assert_eq!(report.comment_claims, 1);

    // The report persists with the manifest across a save/load round
    // trip (§6.4: persisted with the manifest).
    manifest.record_fidelity_report().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    manifest.save_atomic(&path).unwrap();
    let reloaded = Manifest::load(&path).unwrap();
    let persisted = reloaded.fidelity_report.as_ref().expect("report persisted");
    assert!(persisted.reconciles());
    assert_eq!(persisted.total_objects, 2);
    assert_eq!(persisted.non_clean.len(), 1);
}

#[test]
fn fidelity_report_refuses_unreconciled_manifest_13525() {
    let mut manifest = fixture_manifest();
    manifest.assign_page_identities(&mut mint).unwrap();
    // Drop a progress row: the manifest no longer accounts for every
    // inventory object, so report generation must fail loudly (§6.4:
    // a report that does not reconcile is a failed import).
    manifest.objects.remove(MESSY);
    let error = format!("{:?}", generate_report(&manifest).unwrap_err());
    assert!(error.contains("reconcile"), "unexpected error: {error}");
}
