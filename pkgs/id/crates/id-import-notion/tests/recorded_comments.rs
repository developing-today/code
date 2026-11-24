//! Slice 4 (#13524) acceptance over recorded API responses: comment
//! claims (§2.4, §8 check 7) — every fixture comment the acquisition
//! path could see exists as a claim on the expected note/block, with
//! replies chained by manifest-recorded parent comment ID and resolved
//! discussions carrying the resolution fact; every page it could not
//! see is listed in the comments-coverage accounting.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use id_import_notion::comments::{
    CommentConversion, CommentsCoverage, convert_comments,
};
use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION, NotionApi};
use serde_json::{Value, json};

const ALPHA: &str = "11111111-1111-1111-1111-111111111111";
const BETA: &str = "22222222-2222-2222-2222-222222222222";
const GAMMA: &str = "33333333-3333-3333-3333-333333333333";
const ANCHOR_BLOCK: &str = "44444444-4444-4444-4444-444444444444";

fn list(results: Vec<Value>) -> Value {
    json!({"object": "list", "results": results, "has_more": false})
}

fn comment(
    id: &str,
    discussion_id: &str,
    parent: Value,
    author_id: &str,
    author_name: &str,
    created_time: &str,
    body: &str,
    resolved: Option<bool>,
) -> Value {
    let mut value = json!({
        "object": "comment",
        "id": id,
        "parent": parent,
        "discussion_id": discussion_id,
        "created_time": created_time,
        "last_edited_time": created_time,
        "created_by": {"object": "user", "id": author_id, "name": author_name, "type": "person"},
        "rich_text": [{"type": "text", "plain_text": body, "text": {"content": body}}],
    });
    if let Some(resolved) = resolved {
        value["resolved"] = json!(resolved);
    }
    value
}

fn page_parent(page_id: &str) -> Value {
    json!({"type": "page_id", "page_id": page_id})
}

fn block_parent(block_id: &str) -> Value {
    json!({"type": "block_id", "block_id": block_id})
}

struct FixtureApi {
    comment_calls: Vec<String>,
}

impl FixtureApi {
    fn new() -> Self {
        Self {
            comment_calls: Vec::new(),
        }
    }
}

impl NotionApi for FixtureApi {
    fn notion_version(&self) -> &str {
        NOTION_API_VERSION
    }
    fn search(&mut self, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn block_children(&mut self, _block_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn database(&mut self, _database_id: &str) -> Result<Value> {
        bail!("unused in comments fixtures")
    }
    fn data_source(&mut self, _data_source_id: &str) -> Result<Value> {
        bail!("unused in comments fixtures")
    }
    fn query_data_source(&mut self, _data_source_id: &str, _cursor: Option<&str>) -> Result<Value> {
        Ok(list(vec![]))
    }
    fn comments(&mut self, page_id: &str, cursor: Option<&str>) -> Result<Value> {
        self.comment_calls.push(page_id.to_owned());
        if page_id == GAMMA {
            return Ok(list(vec![]));
        }
        if page_id != ALPHA {
            bail!("comments fetched for a page without fixture comments: {page_id}");
        }
        Ok(match cursor {
            None => json!({
                "object": "list",
                "results": [
                    // Reply arrives before its root in API order: the
                    // chain is derived from created_time, not position.
                    comment("c2", "d1", page_parent(ALPHA), "user-bo", "Bo", "2026-11-20T10:05:00Z", "Agreed", None),
                    comment("c1", "d1", page_parent(ALPHA), "user-ada", "Ada", "2026-11-20T10:00:00Z", "First thought", None),
                    comment("c3", "d2", block_parent(ANCHOR_BLOCK), "user-cy", "Cy", "2026-11-20T10:07:00Z", "On this block", None),
                ],
                "has_more": true,
                "next_cursor": "page-2",
            }),
            Some("page-2") => list(vec![
                comment("c4", "d3", page_parent(ALPHA), "user-ada", "Ada", "2026-11-20T10:10:00Z", "Fixed", Some(true)),
                comment("c5", "d1", page_parent(ALPHA), "user-ada", "Ada", "2026-11-20T10:12:00Z", "Also this", None),
            ]),
            _ => bail!("unexpected comments cursor"),
        })
    }
    fn comments_available(&mut self, page_id: &str) -> Result<bool> {
        Ok(page_id != BETA)
    }
    fn visible_share_ids(&self) -> Vec<String> {
        vec![ALPHA.to_owned()]
    }
    fn workspace_edition(&self) -> Option<String> {
        None
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
        page(GAMMA, "Gamma"),
        json!({
            "object": "block",
            "id": ANCHOR_BLOCK,
            "parent": {"type": "page_id", "page_id": ALPHA},
            "type": "paragraph",
            "paragraph": {"rich_text": []},
        }),
    ]);
    let inventory = Inventory::from_recorded_responses(
        &[search],
        &BTreeMap::new(),
        vec![],
        BTreeMap::from([
            (ALPHA.to_owned(), true),
            (BETA.to_owned(), false),
            (GAMMA.to_owned(), true),
        ]),
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
fn recorded_comments_become_claims_with_coverage_accounting() {
    let mut manifest = fixture_manifest();
    let mut api = FixtureApi::new();

    // Conversion before the identity pass refuses, per the §3 two-pass rule.
    let refused = convert_comments(&mut api, &mut manifest, ALPHA);
    assert!(refused.is_err());

    assert_eq!(manifest.assign_page_identities(&mut mint).unwrap(), 3);

    let conversion: CommentConversion =
        convert_comments(&mut api, &mut manifest, ALPHA).unwrap();
    assert_eq!(conversion.page_id, ALPHA);
    assert_eq!(conversion.note_id, format!("note-{ALPHA}"));
    assert_eq!(conversion.coverage, CommentsCoverage::Covered);
    assert_eq!(conversion.claims.len(), 5);

    // Discussion d1 chains by created_time: c1 is the root even
    // though c2 arrived first in API order; c5 (second page) chains
    // to the same root.
    let by_id = |id: &str| {
        conversion
            .claims
            .iter()
            .find(|claim| claim.comment_id == id)
            .with_context(|| format!("missing claim {id}"))
            .unwrap()
    };
    let c1 = by_id("c1").clone();
    assert_eq!(c1.parent_comment_id, None);
    assert_eq!(c1.author_id, "user-ada");
    assert_eq!(c1.author_name.as_deref(), Some("Ada"));
    assert_eq!(c1.created_time, "2026-11-20T10:00:00Z");
    assert_eq!(c1.body, "First thought");
    assert!(!c1.resolved);
    assert_eq!(c1.note_id, format!("note-{ALPHA}"));
    assert_eq!(c1.block_id, None);
    assert_eq!(by_id("c2").parent_comment_id.as_deref(), Some("c1"));
    assert_eq!(by_id("c2").body, "Agreed");
    assert_eq!(by_id("c5").parent_comment_id.as_deref(), Some("c1"));
    // Block-anchored comment names the block identity.
    let c3 = by_id("c3");
    assert_eq!(c3.block_id.as_deref(), Some(ANCHOR_BLOCK));
    assert_eq!(c3.discussion_id, "d2");
    assert_eq!(c3.parent_comment_id, None);
    // Resolved discussion carries the resolution as a recorded fact.
    let c4 = by_id("c4");
    assert!(c4.resolved);
    assert_eq!(c4.discussion_id, "d3");
    assert_eq!(c4.parent_comment_id, None);

    // Manifest state: claims and coverage recorded per page.
    assert_eq!(manifest.comment_claims[ALPHA].len(), 5);
    assert_eq!(
        manifest.comments_coverage[ALPHA],
        CommentsCoverage::Covered
    );

    // Beta: the acquisition path cannot see comments, so the page is
    // listed as uncovered and no comments endpoint call is made.
    let calls_before = api.comment_calls.len();
    let beta = convert_comments(&mut api, &mut manifest, BETA).unwrap();
    assert_eq!(beta.claims.len(), 0);
    assert_eq!(beta.coverage, CommentsCoverage::Unavailable);
    assert_eq!(api.comment_calls.len(), calls_before);
    assert_eq!(
        manifest.comments_coverage[BETA],
        CommentsCoverage::Unavailable
    );
    assert!(manifest.comment_claims[BETA].is_empty());

    // Gamma: available but no comments — covered with an empty claim set,
    // distinct from Beta's unavailable.
    let gamma = convert_comments(&mut api, &mut manifest, GAMMA).unwrap();
    assert_eq!(gamma.claims.len(), 0);
    assert_eq!(gamma.coverage, CommentsCoverage::Covered);
    assert_eq!(
        manifest.comments_coverage[GAMMA],
        CommentsCoverage::Covered
    );
    assert!(manifest.comment_claims[GAMMA].is_empty());

    // §8 check 3 at conversion level: re-running over the unchanged
    // corpus reproduces identical claims and coverage.
    let claims_before = manifest.comment_claims.clone();
    let coverage_before = manifest.comments_coverage.clone();
    let rerun = convert_comments(&mut api, &mut manifest, ALPHA).unwrap();
    assert_eq!(rerun.claims, conversion.claims);
    assert_eq!(manifest.comment_claims, claims_before);
    assert_eq!(manifest.comments_coverage, coverage_before);

    // Manifest round-trips, and a pre-slice-4 manifest (no comment
    // fields) loads with empty comment maps.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    manifest.save_atomic(&path).unwrap();
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded, manifest);
    let mut encoded = serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap();
    encoded.as_object_mut().unwrap().remove("comment_claims");
    encoded
        .as_object_mut()
        .unwrap()
        .remove("comments_coverage");
    let old_path = dir.path().join("manifest-old.json");
    std::fs::write(&old_path, serde_json::to_string(&encoded).unwrap()).unwrap();
    let old = Manifest::load(&old_path).unwrap();
    assert!(old.comment_claims.is_empty());
    assert!(old.comments_coverage.is_empty());
}

#[test]
fn converting_comments_for_unknown_or_non_page_objects_fails() {
    let mut manifest = fixture_manifest();
    let mut api = FixtureApi::new();
    assert!(convert_comments(&mut api, &mut manifest, "page-missing").is_err());
    assert!(convert_comments(&mut api, &mut manifest, ANCHOR_BLOCK).is_err());
}
