//! Page/block core for the Notion importer, slice 2 (#13522).
//!
//! Converts inventoried pages into the note revision content form of
//! design #6436: §2.1 (pages are notes, blocks are content inside them),
//! §2.5 (every block is clean, degraded, or unsupported — unsupported is
//! preserved verbatim, never dropped), §3 (two-pass link rewriting:
//! [`Manifest::assign_page_identities`] runs over the whole inventory
//! before any page converts, so forward references resolve), and §4
//! (files are fetched during the run through an injected boundary and
//! stored as content-addressed blobs).
//!
//! The full degradation table (toggle/callout/table structure, synced
//! blocks, embeds) is slice 4 (#13524): in this slice the degraded and
//! unsupported classes are preserved verbatim with their children, so no
//! content is lost and no block identity changes when slice 4 upgrades
//! their conversion. Database notes and relation claims are slice 3
//! (#13523); a `child_database` block emits its reference here, with
//! the database's note ID resolved from slice 3's identity pass.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    FileBlobRecord, Manifest, NotionApi, Outcome, Stage, crawl_pages, required_string,
    sanitized_source,
};

/// Fetch the bytes behind a fresh, expiring Notion file URL (§4).
/// Implementations receive the URL only at fetch time and must not log or
/// persist it: signed URLs are temporary credentials (§6.5).
pub trait FileFetcher {
    fn fetch(&mut self, url: &str) -> Result<Vec<u8>>;
}

/// The §2.5 class of a block type: clean (a model content form exists),
/// degraded (imported with a named, counted loss), or unsupported
/// (preserved verbatim as a source blob, never dropped).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockClass {
    Clean,
    Degraded,
    Unsupported,
}

/// Classify a Notion block type per the §2.5 table. Unknown and future
/// block types are unsupported by default — the forward-compatibility
/// guarantee that a Notion release cannot make the importer drop content.
pub fn classify_block(block_type: &str) -> BlockClass {
    match block_type {
        // §2.5 clean rows, plus media (clean via §4) and child page /
        // database blocks (clean via §2.1 / §2.2).
        "paragraph" | "heading_1" | "heading_2" | "heading_3" | "bulleted_list_item"
        | "numbered_list_item" | "to_do" | "quote" | "code" | "divider" | "bookmark" | "image"
        | "video" | "file" | "audio" | "pdf" | "child_page" | "child_database" => BlockClass::Clean,
        // §2.5 clean-to-degraded and degraded rows; structural conversion
        // is slice 4, verbatim preservation is this slice.
        "toggle" | "callout" | "column_list" | "column" | "table" | "table_row" | "equation"
        | "embed" | "link_preview" | "synced_block" => BlockClass::Degraded,
        _ => BlockClass::Unsupported,
    }
}

/// One rich-text span in the content form. Annotations are flattened from
/// Notion's annotation object; a span's link is a typed reference to a
/// note when §3 rewriting resolved it, or the verbatim external URL.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RichSpan {
    pub text: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub strikethrough: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub code: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<SpanLink>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpanLink {
    /// A link outside the import set, kept verbatim (§3).
    External { url: String },
    /// A typed reference to an imported note, with the target block's
    /// identity when the source link named one (§3 block links).
    Note {
        note_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        block_id: Option<String>,
    },
}

/// Where a typed reference was authored (§2.3: relations, mentions, and
/// links are one mechanism, differing only in surface).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceSurface {
    ChildPage,
    ChildDatabase,
    InlineLink,
    Mention,
}

/// A typed reference emitted by conversion. Backlinks are the reverse
/// view of these and are computed, never stored twice (§3).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TypedReference {
    /// Block the reference sits in (for a child page/database, the block
    /// itself).
    pub from_block_id: String,
    /// Notion UUID of the target page or database.
    pub target_id: String,
    /// Destination note ID when the target's identity is assigned. Pages
    /// and databases are assigned in the identity passes; a
    /// child-database reference carries `None` only when its database
    /// is outside the import set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_note_id: Option<String>,
    /// Block selector for block links (§3): the revision-pin slot is
    /// never used for blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_block_id: Option<String>,
    pub surface: ReferenceSurface,
}

/// A link or mention whose target is outside the import set (§3): the
/// content keeps the verbatim URL or text, and the item is counted here
/// for the fidelity report instead of being rewritten or dropped.
/// `target_id` is the normalized (dashless, lowercase) Notion UUID.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Externalised {
    pub from_block_id: String,
    pub target_id: String,
    pub surface: ReferenceSurface,
}

/// A file that could not be fetched during the run (§4): the page is
/// degraded, never aborted, and the reason never contains the expiring
/// file URL.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MissingFile {
    pub block_id: String,
    pub reason: String,
}

/// One block in the revision content form. `block_id` is the Notion
/// block UUID, which is also the block's stable identity within the
/// note (§2.1); the manifest records it in `block_id_to_identity`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContentBlock {
    pub block_id: String,
    /// The Notion block type, verbatim.
    pub block_type: String,
    pub class: BlockClass,
    /// Text content for text-bearing kinds; the caption for media and
    /// bookmark blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rich_text: Vec<RichSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ContentBlock>,
    /// To-do checked state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<bool>,
    /// Code block language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Bookmark target or external media URL. Never an expiring Notion
    /// file URL (§4): hosted files are fetched and named by `blob_hash`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Title of a child page or child database block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// BLAKE3-256 hex of the fetched file bytes (the model's
    /// `raw-blake3-256` content profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob_hash: Option<String>,
    /// Why this block's file could not be fetched (§4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_file: Option<String>,
    /// The verbatim source block (with children embedded, file URLs
    /// stripped) for degraded and unsupported blocks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbatim: Option<Value>,
}

/// A page converted to the revision content form: one logical note's
/// initial revision content (§2.1).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PageContent {
    pub page_id: String,
    pub note_id: String,
    pub title: String,
    pub blocks: Vec<ContentBlock>,
}

/// Per-class block counts for one converted page (the fidelity report's
/// §2.5 accounting input; report generation itself is slice 5).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassCounts {
    pub clean: usize,
    pub degraded: usize,
    pub unsupported: usize,
}

/// The result of converting one page: its content form plus everything
/// the manifest and the fidelity report must account for.
#[derive(Clone, Debug, PartialEq)]
pub struct PageConversion {
    pub content: PageContent,
    pub references: Vec<TypedReference>,
    pub externalised: Vec<Externalised>,
    pub missing_files: Vec<MissingFile>,
    /// Blobs fetched for this page, keyed by content hash. Duplicate
    /// bytes appear once however many blocks referenced them (§4).
    pub blobs: BTreeMap<String, Vec<u8>>,
    pub counts: ClassCounts,
}

/// Convert one inventoried page to its content form, updating the
/// manifest's block identities, file map, and staged progress.
///
/// The caller must have run [`Manifest::assign_page_identities`] first
/// (the §3 two-pass rule); converting a page with no assigned note ID
/// fails rather than minting identity mid-content. Conversion is safe
/// to re-run: identities already recorded are kept, and progress never
/// rewinds (§6.3).
pub fn convert_page(
    api: &mut impl NotionApi,
    fetcher: &mut impl FileFetcher,
    manifest: &mut Manifest,
    page_id: &str,
) -> Result<PageConversion> {
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
    let title = page_title(&object.source);

    let mut visited = BTreeSet::new();
    let raw_blocks = fetch_raw_tree(api, page_id, &mut visited)?;
    let mut converter = Converter {
        page_lookup: manifest
            .page_id_to_note_id
            .keys()
            .map(|id| (normalize_uuid(id), id.clone()))
            .collect(),
        block_lookup: manifest
            .inventory
            .objects
            .values()
            .filter(|object| object.kind == "block")
            .map(|object| (normalize_uuid(&object.id), object.id.clone()))
            .collect(),
        manifest,
        note_id: note_id.clone(),
        references: Vec::new(),
        externalised: Vec::new(),
        missing_files: Vec::new(),
        blobs: BTreeMap::new(),
        counts: ClassCounts::default(),
    };
    let mut blocks = Vec::with_capacity(raw_blocks.len());
    for raw in &raw_blocks {
        blocks.push(converter.convert_block(fetcher, raw)?);
    }
    let Converter {
        references,
        externalised,
        missing_files,
        blobs,
        counts,
        manifest,
        ..
    } = converter;

    let files_outcome = if missing_files.is_empty() {
        Outcome::Clean
    } else {
        Outcome::Degraded
    };
    let outcome = if missing_files.is_empty() && counts.degraded == 0 && counts.unsupported == 0 {
        Outcome::Clean
    } else {
        Outcome::Degraded
    };
    let reason = (outcome == Outcome::Degraded).then(|| {
        format!(
            "{} degraded block(s), {} unsupported block(s), {} missing file(s)",
            counts.degraded,
            counts.unsupported,
            missing_files.len()
        )
    });
    advance(manifest, page_id, Stage::Files, files_outcome, None)?;
    advance(
        manifest,
        page_id,
        Stage::Content,
        outcome.clone(),
        reason.clone(),
    )?;
    advance(manifest, page_id, Stage::References, outcome, reason)?;

    Ok(PageConversion {
        content: PageContent {
            page_id: page_id.to_owned(),
            note_id,
            title,
            blocks,
        },
        references,
        externalised,
        missing_files,
        blobs,
        counts,
    })
}

/// Record staged progress without rewinding it (§6.3): re-converting a
/// page whose checkpoint already sits at a later stage refreshes the
/// conversion artifacts but leaves the checkpoint alone.
fn advance(
    manifest: &mut Manifest,
    object_id: &str,
    stage: Stage,
    outcome: Outcome,
    reason: Option<String>,
) -> Result<()> {
    let rank = |value: &Stage| match value {
        Stage::Inventory => 0,
        Stage::Files => 1,
        Stage::Content => 2,
        Stage::References => 3,
        Stage::Report => 4,
    };
    let current = manifest
        .objects
        .get(object_id)
        .with_context(|| format!("object {object_id} is not in the manifest inventory"))?;
    if rank(&stage) >= rank(&current.stage) {
        manifest.record_progress(object_id, stage, outcome, reason)?;
    }
    Ok(())
}

/// Fetch a block-children subtree with sibling order preserved and each
/// descended block's children embedded under a `children` key, so
/// conversion (and verbatim preservation) never re-reads the API.
/// Child page and child database blocks are not descended: their
/// content belongs to their own notes. A repeated block ID fails
/// closed, like slice 1's pagination guard.
fn fetch_raw_tree(
    api: &mut impl NotionApi,
    parent_id: &str,
    visited: &mut BTreeSet<String>,
) -> Result<Vec<Value>> {
    let pages = crawl_pages(|cursor| api.block_children(parent_id, cursor))?;
    let mut blocks = Vec::new();
    for page in pages {
        let results = page
            .get("results")
            .and_then(Value::as_array)
            .context("Notion block-children response is missing results array")?;
        for raw in results {
            let mut raw = raw.clone();
            let id = required_string(&raw, "id")?;
            if !visited.insert(id.clone()) {
                bail!("Notion block tree repeats block {id}");
            }
            let block_type = raw.get("type").and_then(Value::as_str).unwrap_or("");
            let has_children = raw
                .get("has_children")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if has_children && block_type != "child_page" && block_type != "child_database" {
                let children = fetch_raw_tree(api, &id, visited)?;
                raw["children"] = Value::Array(children);
            }
            blocks.push(raw);
        }
    }
    Ok(blocks)
}

/// Extract the page title from an inventoried page object: the plain
/// text of its title property (§2.2's title-property rule, applied to
/// pages). Pages without a title property yield an empty title.
fn page_title(page_source: &Value) -> String {
    let Some(properties) = page_source.get("properties").and_then(Value::as_object) else {
        return String::new();
    };
    for property in properties.values() {
        if property.get("type").and_then(Value::as_str) == Some("title") {
            let mut title = String::new();
            if let Some(items) = property.get("title").and_then(Value::as_array) {
                for item in items {
                    if let Some(text) = item.get("plain_text").and_then(Value::as_str) {
                        title.push_str(text);
                    }
                }
            }
            return title;
        }
    }
    String::new()
}

/// Normalize a Notion UUID for comparison: dashless, lowercase. The API
/// returns dashed UUIDs; URLs carry both dashed and dashless forms.
pub fn normalize_uuid(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// Find the first UUID in `text` — dashed (8-4-4-4-12) or a bare
/// 32-hex-digit run — and return it normalized.
fn find_uuid(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    for start in 0..chars.len() {
        if start + 36 <= chars.len() {
            let candidate = &chars[start..start + 36];
            if [8, 13, 18, 23].iter().all(|&i| candidate[i] == '-')
                && candidate
                    .iter()
                    .enumerate()
                    .all(|(i, c)| [8, 13, 18, 23].contains(&i) || is_hex(*c))
            {
                let candidate: String = candidate.iter().collect();
                return Some(normalize_uuid(&candidate));
            }
        }
        if start + 32 <= chars.len()
            && chars[start..start + 32].iter().all(|c| is_hex(*c))
            && (start + 32 == chars.len() || !is_hex(chars[start + 32]))
        {
            let candidate: String = chars[start..start + 32].iter().collect();
            return Some(normalize_uuid(&candidate));
        }
    }
    None
}

/// Parse a Notion page/block URL into its (page UUID, optional block
/// UUID), both normalized. Returns `None` when the URL names no UUID —
/// an ordinary external link. Notion URLs put the page UUID last in the
/// path and the block UUID in the `#` fragment.
pub fn extract_notion_link(url: &str) -> Option<(String, Option<String>)> {
    let (main, fragment) = match url.split_once('#') {
        Some((main, fragment)) => (main, Some(fragment)),
        None => (url, None),
    };
    let page = find_uuid(main)?;
    let block = fragment.and_then(find_uuid);
    Some((normalize_uuid(&page), block))
}

struct Converter<'a> {
    manifest: &'a mut Manifest,
    note_id: String,
    page_lookup: BTreeMap<String, String>,
    block_lookup: BTreeMap<String, String>,
    references: Vec<TypedReference>,
    externalised: Vec<Externalised>,
    missing_files: Vec<MissingFile>,
    blobs: BTreeMap<String, Vec<u8>>,
    counts: ClassCounts,
}

impl Converter<'_> {
    fn convert_block(
        &mut self,
        fetcher: &mut impl FileFetcher,
        raw: &Value,
    ) -> Result<ContentBlock> {
        let block_id = required_string(raw, "id")?;
        let block_type = required_string(raw, "type")
            .with_context(|| format!("Notion block {block_id} is missing its type"))?;
        let class = classify_block(&block_type);
        match self.manifest.block_id_to_identity.get(&block_id) {
            Some((note_id, _)) if *note_id != self.note_id => {
                bail!("block {block_id} is already assigned to note {note_id}");
            }
            _ => {
                self.manifest
                    .block_id_to_identity
                    .insert(block_id.clone(), (self.note_id.clone(), block_id.clone()));
            }
        }
        match class {
            BlockClass::Clean => self.counts.clean += 1,
            BlockClass::Degraded => self.counts.degraded += 1,
            BlockClass::Unsupported => self.counts.unsupported += 1,
        }

        let mut block = ContentBlock {
            block_id: block_id.clone(),
            block_type: block_type.clone(),
            class,
            rich_text: Vec::new(),
            children: Vec::new(),
            checked: None,
            language: None,
            url: None,
            title: None,
            blob_hash: None,
            missing_file: None,
            verbatim: None,
        };
        if class != BlockClass::Clean {
            // Degraded and unsupported blocks are preserved verbatim in
            // this slice (children were embedded by the tree fetch);
            // slice 4 upgrades their structural conversion. File URLs
            // are stripped: they are expiring credentials (§6.5).
            block.verbatim = Some(sanitized_source(raw.clone()));
            return Ok(block);
        }

        let payload = raw.get(block_type.as_str()).cloned().unwrap_or(Value::Null);
        match block_type.as_str() {
            "paragraph" | "heading_1" | "heading_2" | "heading_3" | "bulleted_list_item"
            | "numbered_list_item" | "to_do" | "quote" | "code" => {
                block.rich_text = self.convert_rich_text(payload.get("rich_text"), &block_id);
                if block_type == "to_do" {
                    block.checked = payload.get("checked").and_then(Value::as_bool);
                }
                if block_type == "code" {
                    block.language = payload
                        .get("language")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
            }
            "divider" => {}
            "bookmark" => {
                block.url = payload
                    .get("url")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                block.rich_text = self.convert_rich_text(payload.get("caption"), &block_id);
            }
            "image" | "video" | "file" | "audio" | "pdf" => {
                self.convert_media(fetcher, &mut block, &payload, &block_id)?;
            }
            "child_page" => {
                block.title = payload
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let target_note_id = self
                    .manifest
                    .page_id_to_note_id
                    .get(&block_id)
                    .with_context(|| {
                        format!(
                            "child page {block_id} has no assigned note id; \
                             run assign_page_identities first"
                        )
                    })?
                    .clone();
                self.references.push(TypedReference {
                    from_block_id: block_id.clone(),
                    target_id: block_id.clone(),
                    target_note_id: Some(target_note_id),
                    target_block_id: None,
                    surface: ReferenceSurface::ChildPage,
                });
            }
            "child_database" => {
                block.title = payload
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                // The block's ID is the database's ID (the slice 1
                // inventory rule); slice 3's identity pass assigns
                // its note ID. A database in the inventory without
                // an assigned note ID means the pass did not run —
                // fail closed, as for child pages. A database
                // outside the import set keeps `None`: the reference
                // still records the target ID.
                let target_note_id = self.manifest.database_id_to_note_id.get(&block_id).cloned();
                if target_note_id.is_none()
                    && self
                        .manifest
                        .inventory
                        .objects
                        .get(&block_id)
                        .is_some_and(|object| object.kind == "database")
                {
                    bail!(
                        "child database {block_id} has no assigned note id; \
                         run assign_database_identities first"
                    );
                }
                self.references.push(TypedReference {
                    from_block_id: block_id.clone(),
                    target_id: block_id.clone(),
                    target_note_id,
                    target_block_id: None,
                    surface: ReferenceSurface::ChildDatabase,
                });
            }
            _ => {
                // classify_block and this dispatch disagree: fail closed
                // rather than silently dropping a clean-class block.
                bail!("no conversion for clean block type {block_type}");
            }
        }
        if let Some(children) = raw.get("children").and_then(Value::as_array) {
            for child in children {
                block.children.push(self.convert_block(fetcher, child)?);
            }
        }
        Ok(block)
    }

    fn convert_media(
        &mut self,
        fetcher: &mut impl FileFetcher,
        block: &mut ContentBlock,
        payload: &Value,
        block_id: &str,
    ) -> Result<()> {
        block.rich_text = self.convert_rich_text(payload.get("caption"), block_id);
        match payload.get("type").and_then(Value::as_str) {
            Some("file") => {
                let url = payload
                    .get("file")
                    .and_then(|file| file.get("url"))
                    .and_then(Value::as_str);
                match url {
                    Some(url) => match fetcher.fetch(url) {
                        Ok(bytes) => {
                            let hash = blake3::hash(&bytes).to_hex().as_str().to_owned();
                            self.manifest.file_blobs.insert(
                                block_id.to_owned(),
                                FileBlobRecord {
                                    blob_hash: hash.clone(),
                                    size_bytes: bytes.len() as u64,
                                },
                            );
                            self.blobs.entry(hash.clone()).or_insert(bytes);
                            block.blob_hash = Some(hash);
                        }
                        Err(error) => {
                            // The reason is persisted with the report, so
                            // the expiring URL is stripped from it (§6.5).
                            let reason =
                                format!("fetch failed: {error:#}").replace(url, "<file-url>");
                            self.missing_files.push(MissingFile {
                                block_id: block_id.to_owned(),
                                reason: reason.clone(),
                            });
                            block.missing_file = Some(reason);
                        }
                    },
                    None => {
                        let reason = "file URL absent from the fresh API response".to_owned();
                        self.missing_files.push(MissingFile {
                            block_id: block_id.to_owned(),
                            reason: reason.clone(),
                        });
                        block.missing_file = Some(reason);
                    }
                }
            }
            Some("external") => {
                block.url = payload
                    .get("external")
                    .and_then(|external| external.get("url"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            _ => {
                bail!("media block {block_id} has a missing or unknown payload type");
            }
        }
        Ok(())
    }

    fn convert_rich_text(&mut self, items: Option<&Value>, from_block_id: &str) -> Vec<RichSpan> {
        let Some(items) = items.and_then(Value::as_array) else {
            return Vec::new();
        };
        items
            .iter()
            .map(|item| self.convert_span(item, from_block_id))
            .collect()
    }

    fn convert_span(&mut self, item: &Value, from_block_id: &str) -> RichSpan {
        let text = item
            .get("plain_text")
            .and_then(Value::as_str)
            .or_else(|| item.pointer("/text/content").and_then(Value::as_str))
            .unwrap_or("")
            .to_owned();
        let annotations = item.get("annotations");
        let flag = |key: &str| {
            annotations
                .and_then(|value| value.get(key))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let mut span = RichSpan {
            text,
            bold: flag("bold"),
            italic: flag("italic"),
            strikethrough: flag("strikethrough"),
            underline: flag("underline"),
            code: flag("code"),
            link: None,
        };
        match item.get("type").and_then(Value::as_str) {
            Some("mention") => {
                self.convert_mention(item, from_block_id, &mut span);
            }
            Some("equation") => {
                span.code = true;
            }
            _ => {
                let url = item
                    .pointer("/text/link/url")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("href").and_then(Value::as_str));
                if let Some(url) = url {
                    span.link = Some(self.resolve_link(url, from_block_id));
                }
            }
        }
        span
    }

    fn convert_mention(&mut self, item: &Value, from_block_id: &str, span: &mut RichSpan) {
        let mention = item.get("mention");
        if mention
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            != Some("page")
        {
            // User and date mentions are plain text (people are
            // provenance, never principals, §7). Database mentions
            // resolve with slice 3's database notes.
            return;
        }
        let Some(target_id) = mention
            .and_then(|value| value.get("page"))
            .and_then(|page| page.get("id"))
            .and_then(Value::as_str)
        else {
            return;
        };
        match self.manifest.page_id_to_note_id.get(target_id) {
            Some(note_id) => {
                let note_id = note_id.clone();
                self.references.push(TypedReference {
                    from_block_id: from_block_id.to_owned(),
                    target_id: target_id.to_owned(),
                    target_note_id: Some(note_id.clone()),
                    target_block_id: None,
                    surface: ReferenceSurface::Mention,
                });
                span.link = Some(SpanLink::Note {
                    note_id,
                    block_id: None,
                });
            }
            None => {
                self.externalised.push(Externalised {
                    from_block_id: from_block_id.to_owned(),
                    target_id: normalize_uuid(target_id),
                    surface: ReferenceSurface::Mention,
                });
            }
        }
    }

    /// Rewrite one link URL per §3: a Notion link to an imported page
    /// becomes a typed reference (with its block selector when the
    /// fragment names an inventoried block of that page); a Notion link
    /// outside the import set stays verbatim and is counted externalised;
    /// any other URL is an ordinary external link.
    fn resolve_link(&mut self, url: &str, from_block_id: &str) -> SpanLink {
        let Some((page_token, block_token)) = extract_notion_link(url) else {
            return SpanLink::External {
                url: url.to_owned(),
            };
        };
        let Some(target_page) = self.page_lookup.get(&page_token).cloned() else {
            self.externalised.push(Externalised {
                from_block_id: from_block_id.to_owned(),
                target_id: page_token,
                surface: ReferenceSurface::InlineLink,
            });
            return SpanLink::External {
                url: url.to_owned(),
            };
        };
        let note_id = self.manifest.page_id_to_note_id[&target_page].clone();
        let mut target_block_id = None;
        if let Some(block_token) = block_token {
            match self.block_lookup.get(&block_token) {
                Some(api_block)
                    if self.owning_page(api_block).as_deref() == Some(target_page.as_str()) =>
                {
                    target_block_id = Some(api_block.clone());
                }
                _ => {
                    self.externalised.push(Externalised {
                        from_block_id: from_block_id.to_owned(),
                        target_id: block_token,
                        surface: ReferenceSurface::InlineLink,
                    });
                }
            }
        }
        self.references.push(TypedReference {
            from_block_id: from_block_id.to_owned(),
            target_id: target_page,
            target_note_id: Some(note_id.clone()),
            target_block_id: target_block_id.clone(),
            surface: ReferenceSurface::InlineLink,
        });
        SpanLink::Note {
            note_id,
            block_id: target_block_id,
        }
    }

    /// The page a block belongs to, per the inventory's parent chain.
    fn owning_page(&self, block_id: &str) -> Option<String> {
        let mut current = block_id.to_owned();
        let mut guard = BTreeSet::new();
        loop {
            if !guard.insert(current.clone()) {
                return None;
            }
            let object = self.manifest.inventory.objects.get(&current)?;
            if object.kind == "page" {
                return Some(current);
            }
            current = object.parent_id.clone()?;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_design_table() {
        for clean in [
            "paragraph",
            "heading_2",
            "to_do",
            "code",
            "image",
            "child_page",
        ] {
            assert_eq!(classify_block(clean), BlockClass::Clean, "{clean}");
        }
        for degraded in [
            "toggle",
            "callout",
            "table",
            "equation",
            "synced_block",
            "embed",
        ] {
            assert_eq!(classify_block(degraded), BlockClass::Degraded, "{degraded}");
        }
        for unsupported in [
            "table_of_contents",
            "breadcrumb",
            "template_button",
            "new_widget",
        ] {
            assert_eq!(
                classify_block(unsupported),
                BlockClass::Unsupported,
                "{unsupported}"
            );
        }
    }

    #[test]
    fn extracts_dashless_page_and_block_uuids() {
        let (page, block) = extract_notion_link(
            "https://www.notion.so/ws/Beta-22222222222222222222222222222222#44444444444444444444444444444444",
        )
        .unwrap();
        assert_eq!(page, "22222222222222222222222222222222");
        assert_eq!(block.as_deref(), Some("44444444444444444444444444444444"));
    }

    #[test]
    fn extracts_dashed_uuids() {
        let (page, block) = extract_notion_link(
            "https://www.notion.so/Beta-22222222-2222-2222-2222-222222222222#44444444-4444-4444-4444-444444444444",
        )
        .unwrap();
        assert_eq!(page, "22222222222222222222222222222222");
        assert_eq!(block.as_deref(), Some("44444444444444444444444444444444"));
    }

    #[test]
    fn plain_external_urls_have_no_uuid() {
        assert_eq!(extract_notion_link("https://example.com/article"), None);
        assert_eq!(extract_notion_link("https://example.com/abcdef"), None);
    }
}
