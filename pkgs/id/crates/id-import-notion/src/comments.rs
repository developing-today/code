//! Comment claims for the Notion importer, slice 4 (#13524).
//!
//! Design #6436 §2.4: discussion comments become comment claims on the
//! note (block-anchored comments name the block identity), with author
//! recorded as Notion user provenance (ID + display name), timestamp,
//! and body blob; replies chain by manifest-recorded parent comment ID;
//! resolved discussions carry the resolution as a recorded provenance
//! fact. Comments the acquisition path cannot see are absent and
//! counted per page in [`CommentsCoverage`] — never silently partial
//! within a page the importer claims to have covered (§8 check 7).
//!
//! Two properties of the Notion comments API shape this module:
//!
//! * The list-comments response carries no parent-comment ID: replies
//!   share a `discussion_id` with their root. The chain is therefore
//!   derived — within one discussion the earliest-created comment is
//!   the root and every later comment chains to it — and recorded
//!   explicitly, so a later reader never re-derives it.
//! * The public list-comments response carries no resolution field.
//!   `resolved` is parsed when a transport supplies it and defaults to
//!   `false`; a transport that cannot see resolution records no
//!   resolved facts rather than inventing them.
//!
//! Comment attachments are blobs (§4) named by the comment claim; the
//! fetch itself is a follow-up slice, not this one.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Manifest, NotionApi, crawl_pages, required_string};

/// Whether the acquisition path could see a page's comments (§2.4).
/// `Covered` means the comments endpoint was queried for the page —
/// including a page with zero comments. `Unavailable` means the path
/// could not see them at all, and the page is listed as such in the
/// coverage accounting instead of silently having no comments.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommentsCoverage {
    Covered,
    Unavailable,
}

/// One discussion comment as a claim on a note (§2.4). The author is
/// Notion user provenance only — people values are never principals
/// (§7 point 4) — and the body is the plain-text concatenation of the
/// comment's rich-text spans.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CommentClaim {
    pub comment_id: String,
    pub note_id: String,
    /// The block this comment is anchored to, when its parent is a
    /// block. The block identity is the Notion block UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    pub discussion_id: String,
    /// Manifest-recorded reply chain: `None` for a discussion root,
    /// the root comment's ID for every later comment in the
    /// discussion. Derived from `discussion_id` + `created_time`
    /// because the API carries no parent-comment ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_comment_id: Option<String>,
    /// Notion user ID of the author (provenance, never a principal).
    pub author_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    pub created_time: String,
    pub body: String,
    /// Resolution state as a recorded source fact. `false` is the
    /// default when the transport cannot see resolution (§2.4).
    #[serde(default)]
    pub resolved: bool,
}

/// The result of converting one page's comments: its claims plus the
/// coverage accounting the fidelity report must carry (§6.4).
#[derive(Clone, Debug, PartialEq)]
pub struct CommentConversion {
    pub page_id: String,
    pub note_id: String,
    pub claims: Vec<CommentClaim>,
    pub coverage: CommentsCoverage,
}

/// Convert one inventoried page's comments to claims on its note,
/// recording coverage in the manifest.
///
/// The caller must have run [`Manifest::assign_page_identities`] first
/// (the §3 two-pass rule); converting a page with no assigned note ID
/// fails rather than minting identity mid-content. A page whose
/// comments the acquisition path cannot see records
/// [`CommentsCoverage::Unavailable`] and yields no claims — and no
/// comments-endpoint call is made. Conversion is safe to re-run: the
/// page's claims and coverage are replaced wholesale, so an unchanged
/// corpus reproduces identical artifacts (§8 check 3).
pub fn convert_comments(
    api: &mut impl NotionApi,
    manifest: &mut Manifest,
    page_id: &str,
) -> Result<CommentConversion> {
    let object = manifest
        .inventory
        .objects
        .get(page_id)
        .with_context(|| format!("page {page_id} is not in the inventory"))?;
    if object.kind != "page" {
        bail!(
            "inventory object {page_id} is a {}, not a page",
            object.kind
        );
    }
    let note_id = manifest
        .page_id_to_note_id
        .get(page_id)
        .with_context(|| {
            format!("page {page_id} has no assigned note id; run assign_page_identities first")
        })?
        .clone();
    let available = manifest
        .inventory
        .comments_available
        .get(page_id)
        .with_context(|| format!("comments availability is not recorded for page {page_id}"))?;
    if !available {
        manifest
            .comment_claims
            .insert(page_id.to_owned(), Vec::new());
        manifest
            .comments_coverage
            .insert(page_id.to_owned(), CommentsCoverage::Unavailable);
        return Ok(CommentConversion {
            page_id: page_id.to_owned(),
            note_id,
            claims: Vec::new(),
            coverage: CommentsCoverage::Unavailable,
        });
    }
    let pages = crawl_pages(|cursor| api.comments(page_id, cursor))?;
    let mut raw_comments = Vec::new();
    for page in pages {
        let results = page
            .get("results")
            .and_then(Value::as_array)
            .context("Notion comments response is missing results array")?;
        for raw in results {
            raw_comments.push(parse_comment(raw, &note_id, page_id, manifest)?);
        }
    }
    let claims = chain_replies(raw_comments)?;
    manifest
        .comment_claims
        .insert(page_id.to_owned(), claims.clone());
    manifest
        .comments_coverage
        .insert(page_id.to_owned(), CommentsCoverage::Covered);
    Ok(CommentConversion {
        page_id: page_id.to_owned(),
        note_id,
        claims,
        coverage: CommentsCoverage::Covered,
    })
}

/// A parsed comment before reply chaining: the root of its discussion
/// is not yet known.
struct ParsedComment {
    claim: CommentClaim,
    /// Position in the fetched sequence; breaks `created_time` ties
    /// deterministically.
    order: usize,
}

fn parse_comment(
    raw: &Value,
    note_id: &str,
    page_id: &str,
    manifest: &Manifest,
) -> Result<ParsedComment> {
    if raw.get("object").and_then(Value::as_str) != Some("comment") {
        bail!("Notion comments response returned a non-comment object");
    }
    let comment_id = required_string(raw, "id")?;
    let discussion_id = required_string(raw, "discussion_id")?;
    let created_time = required_string(raw, "created_time")?;
    let author_id = raw
        .pointer("/created_by/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .with_context(|| format!("Notion comment {comment_id} is missing its author id"))?;
    let author_name = raw
        .pointer("/created_by/name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let rich_text = raw
        .get("rich_text")
        .and_then(Value::as_array)
        .with_context(|| format!("Notion comment {comment_id} is missing rich_text"))?;
    let mut body = String::new();
    for span in rich_text {
        if let Some(text) = span
            .get("plain_text")
            .and_then(Value::as_str)
            .or_else(|| span.pointer("/text/content").and_then(Value::as_str))
        {
            body.push_str(text);
        }
    }
    let parent = raw
        .get("parent")
        .context(format!("Notion comment {comment_id} is missing its parent"))?;
    let block_id = match parent.get("type").and_then(Value::as_str) {
        Some("page_id") => {
            let parent_page = required_string(parent, "page_id")?;
            if parent_page != page_id {
                bail!("Notion comment {comment_id} belongs to page {parent_page}, not {page_id}");
            }
            None
        }
        Some("block_id") => {
            let block = required_string(parent, "block_id")?;
            match manifest.inventory.objects.get(&block) {
                Some(object) if object.kind == "block" => Some(block),
                _ => {
                    bail!("Notion comment {comment_id} is anchored to unknown block {block}");
                }
            }
        }
        _ => {
            bail!("Notion comment {comment_id} has an unknown parent type");
        }
    };
    let resolved = raw
        .get("resolved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ParsedComment {
        claim: CommentClaim {
            comment_id,
            note_id: note_id.to_owned(),
            block_id,
            discussion_id,
            parent_comment_id: None,
            author_id,
            author_name,
            created_time,
            body,
            resolved,
        },
        order: 0,
    })
}

/// Derive the reply chain and order claims deterministically: within
/// each discussion the earliest-created comment is the root and every
/// later comment chains to it; discussions appear in first-seen
/// order and comments within a discussion in creation order. A
/// repeated comment ID fails closed — the same comment imported twice
/// would fork its identity.
fn chain_replies(mut parsed: Vec<ParsedComment>) -> Result<Vec<CommentClaim>> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (order, item) in parsed.iter_mut().enumerate() {
        item.order = order;
        if !seen.insert(item.claim.comment_id.clone()) {
            bail!("Notion comments repeat comment {}", item.claim.comment_id);
        }
    }
    // Stable by created_time: equal timestamps keep fetch order.
    parsed.sort_by(|a, b| a.claim.created_time.cmp(&b.claim.created_time));
    let mut roots: BTreeMap<String, String> = BTreeMap::new();
    for item in &mut parsed {
        match roots.get(&item.claim.discussion_id) {
            Some(root) => {
                item.claim.parent_comment_id = Some(root.clone());
            }
            None => {
                roots.insert(
                    item.claim.discussion_id.clone(),
                    item.claim.comment_id.clone(),
                );
            }
        }
    }
    // Restore first-seen discussion order with creation order inside
    // each discussion: sort by (discussion first-seen order,
    // created_time). The stable sort above already ordered by
    // created_time, so re-sorting by discussion first-seen index
    // keeps that inner order.
    let mut first_seen: BTreeMap<String, usize> = BTreeMap::new();
    for item in &parsed {
        first_seen
            .entry(item.claim.discussion_id.clone())
            .or_insert(item.order);
    }
    parsed.sort_by_key(|item| {
        first_seen
            .get(&item.claim.discussion_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    Ok(parsed.into_iter().map(|item| item.claim).collect())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::Inventory;
    use serde_json::json;

    fn raw_comment(id: &str, discussion: &str, created: &str) -> Value {
        json!({
            "object": "comment",
            "id": id,
            "parent": {"type": "page_id", "page_id": "page-1"},
            "discussion_id": discussion,
            "created_time": created,
            "created_by": {"object": "user", "id": "user-1", "name": "Ada"},
            "rich_text": [{"type": "text", "plain_text": "hi", "text": {"content": "hi"}}],
        })
    }

    fn empty_manifest() -> Manifest {
        Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
            crate::NOTION_API_VERSION.into(),
            Inventory::default(),
        )
        .unwrap()
    }

    #[test]
    fn replies_chain_to_the_earliest_comment_in_the_discussion() {
        let manifest = empty_manifest();
        let later = parse_comment(
            &raw_comment("c-later", "d1", "2026-11-20T10:05:00Z"),
            "note-1",
            "page-1",
            &manifest,
        )
        .unwrap();
        let root = parse_comment(
            &raw_comment("c-root", "d1", "2026-11-20T10:00:00Z"),
            "note-1",
            "page-1",
            &manifest,
        )
        .unwrap();
        let claims = chain_replies(vec![later, root]).unwrap();
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[0].comment_id, "c-root");
        assert_eq!(claims[0].parent_comment_id, None);
        assert_eq!(claims[1].comment_id, "c-later");
        assert_eq!(claims[1].parent_comment_id.as_deref(), Some("c-root"));
    }

    #[test]
    fn comments_fail_closed_on_missing_author_unknown_block_and_duplicates() {
        let manifest = empty_manifest();
        let mut no_author = raw_comment("c1", "d1", "2026-11-20T10:00:00Z");
        no_author["created_by"] = json!({"object": "user", "name": "Ada"});
        assert!(parse_comment(&no_author, "note-1", "page-1", &manifest).is_err());
        let mut foreign_page = raw_comment("c1", "d1", "2026-11-20T10:00:00Z");
        foreign_page["parent"] = json!({"type": "page_id", "page_id": "page-2"});
        assert!(parse_comment(&foreign_page, "note-1", "page-1", &manifest).is_err());
        let mut unknown_block = raw_comment("c1", "d1", "2026-11-20T10:00:00Z");
        unknown_block["parent"] = json!({"type": "block_id", "block_id": "block-missing"});
        assert!(parse_comment(&unknown_block, "note-1", "page-1", &manifest).is_err());
        let mut not_a_comment = raw_comment("c1", "d1", "2026-11-20T10:00:00Z");
        not_a_comment["object"] = json!("page");
        assert!(parse_comment(&not_a_comment, "note-1", "page-1", &manifest).is_err());
        let first = parse_comment(
            &raw_comment("c1", "d1", "2026-11-20T10:00:00Z"),
            "note-1",
            "page-1",
            &manifest,
        )
        .unwrap();
        let second = parse_comment(
            &raw_comment("c1", "d1", "2026-11-20T10:01:00Z"),
            "note-1",
            "page-1",
            &manifest,
        )
        .unwrap();
        assert!(chain_replies(vec![first, second]).is_err());
    }
}
