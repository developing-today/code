//! Durable inventory and resume primitives for the Notion importer, slice 1.
//!
//! The API transport is deliberately injected by callers. In particular, a
//! bearer token is never accepted by or serialized from [`Manifest`].
//!
//! Slice 2 (page/block core, link rewriting, files) lives in [`content`].
//! Slice 4 comment claims (§2.4) live in [`comments`].
//! Slice 3 (databases: note identity and per-data-source schema claims)
//! lives in [`databases`].
//! Slice 3 remainder (row notes, property claims, relations) lives in
//! [`rows`].
//! Slice 4 export reader (§2.5: Markdown+CSV archive, match ledger) lives
//! in [`export`]; export-only degraded mode (§2.5) lives in
//! [`export_only`].

pub mod comments;
pub mod content;
pub mod databases;
pub mod export;
pub mod export_only;
pub mod reimport;
pub mod rows;

#[cfg(unix)]
use std::fs::File;
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Notion API version whose database/data-source response shapes this crate
/// validates and records.
pub const NOTION_API_VERSION: &str = "2025-09-03";

/// Minimal authenticated transport boundary. Implementations receive the
/// credential from the caller and must not retain or log it. The acquisition
/// workflow owns rate limiting/retries; each method returns one raw API page.
pub trait NotionApi {
    /// API version configured on the transport. Crawls require
    /// [`NOTION_API_VERSION`].
    fn notion_version(&self) -> &str;
    /// Fetch one page of workspace search results.
    fn search(&mut self, start_cursor: Option<&str>) -> Result<serde_json::Value>;
    /// Fetch one page of children for a page, block, or inline database.
    fn block_children(
        &mut self,
        block_id: &str,
        start_cursor: Option<&str>,
    ) -> Result<serde_json::Value>;
    /// Retrieve the database container, including its `data_sources` refs.
    fn database(&mut self, database_id: &str) -> Result<serde_json::Value>;
    /// Retrieve one source and its full schema.
    fn data_source(&mut self, data_source_id: &str) -> Result<serde_json::Value>;
    /// Query one page of row pages for a data source.
    fn query_data_source(
        &mut self,
        data_source_id: &str,
        start_cursor: Option<&str>,
    ) -> Result<serde_json::Value>;
    /// Return whether the comments endpoint can be queried for this page.
    fn comments_available(&mut self, page_id: &str) -> Result<bool>;
    /// Fetch one page of comments for a page (`GET /v1/comments`).
    /// Only called for pages whose [`NotionApi::comments_available`]
    /// returned `true`. The default returns an empty list, so transports
    /// and test doubles that predate slice 4 keep compiling; a real
    /// transport must override it.
    fn comments(
        &mut self,
        _page_id: &str,
        _start_cursor: Option<&str>,
    ) -> Result<serde_json::Value> {
        Ok(serde_json::json!({"object":"list","results":[],"has_more":false}))
    }
    /// Return the top-level IDs visible to this integration.
    fn visible_share_ids(&self) -> Vec<String>;
    /// Return the workspace edition when the transport can determine it.
    fn workspace_edition(&self) -> Option<String>;
}

/// Crawl inventory endpoints completely before returning. This function does
/// not create ID objects; it is safe to use as the mandatory inventory-first
/// preflight. An API failure or pagination cycle returns an error, never a
/// partial inventory.
pub fn crawl_inventory(api: &mut impl NotionApi) -> Result<Inventory> {
    if api.notion_version() != NOTION_API_VERSION {
        bail!(
            "Notion transport version {} does not match required {}",
            api.notion_version(),
            NOTION_API_VERSION
        );
    }
    let mut search_pages = crawl_pages(|cursor| api.search(cursor))?;
    let mut pending: Vec<String> = page_ids(&search_pages)?;
    let mut seen_parents = std::collections::BTreeSet::new();
    let mut seen_objects: std::collections::BTreeSet<String> = search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter_map(|value| value.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let mut block_objects = Vec::new();
    let mut inline_database_ids = std::collections::BTreeSet::new();
    crawl_pending_blocks(
        api,
        &mut pending,
        &mut seen_parents,
        &mut seen_objects,
        &mut block_objects,
        &mut inline_database_ids,
    )?;
    let mut database_ids = std::collections::BTreeSet::new();
    for value in search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
    {
        match object_kind(value)?.as_str() {
            "database" => {
                database_ids.insert(required_string(value, "id")?);
            }
            "data_source" => {
                database_ids.insert(data_source_parent_database_id(value)?);
            }
            "block" if is_child_database_block(value) => {
                database_ids.insert(required_string(value, "id")?);
            }
            _ => {}
        }
    }
    let mut data_sources = BTreeMap::new();
    let mut enumerated_databases = std::collections::BTreeSet::new();
    loop {
        database_ids.extend(inline_database_ids.iter().cloned());
        let next_database = database_ids
            .iter()
            .find(|id| !enumerated_databases.contains(*id))
            .cloned();
        let Some(database_id) = next_database else {
            break;
        };
        enumerated_databases.insert(database_id.clone());
        let database = api.database(&database_id)?;
        if required_string(&database, "id")? != database_id || object_kind(&database)? != "database"
        {
            bail!("Notion returned the wrong database for {database_id}");
        }
        let refs = database
            .get("data_sources")
            .and_then(|value| value.as_array())
            .context("Notion database response is missing data_sources array")?;
        if refs.is_empty() {
            bail!("Notion database {database_id} has no data sources");
        }
        let source_ids: Vec<String> = refs
            .iter()
            .map(|source| required_string(source, "id"))
            .collect::<Result<_>>()?;
        let mut source_objects = Vec::with_capacity(source_ids.len());
        for source_id in source_ids {
            let source = api.data_source(&source_id)?;
            if required_string(&source, "id")? != source_id
                || object_kind(&source)? != "data_source"
            {
                bail!("Notion returned the wrong data source for {source_id}");
            }
            let row_pages = crawl_pages(|cursor| api.query_data_source(&source_id, cursor))?;
            for mut row_page in row_pages {
                // Notion stops paginating a query past its row cap with
                // has_more=false and request_status "incomplete"; accepting
                // that would record a silently truncated row inventory.
                if row_page
                    .get("request_status")
                    .and_then(|status| status.get("type"))
                    .and_then(serde_json::Value::as_str)
                    == Some("incomplete")
                {
                    bail!(
                        "Notion returned an incomplete query for data source {source_id} (row cap reached); refusing a truncated inventory"
                    );
                }
                let results = row_page
                    .get_mut("results")
                    .and_then(serde_json::Value::as_array_mut)
                    .context("Notion data-source query response is missing results array")?;
                let mut unique_results = Vec::with_capacity(results.len());
                for row in results.drain(..) {
                    match object_kind(&row)?.as_str() {
                        "page" => {}
                        // A wiki's query lists its child data sources too;
                        // enumerate them through their parent database.
                        "data_source" => {
                            database_ids.insert(data_source_parent_database_id(&row)?);
                            continue;
                        }
                        _ => bail!("Notion data-source query returned a non-page for {source_id}"),
                    }
                    let row_id = required_string(&row, "id")?;
                    if seen_objects.insert(row_id.clone()) {
                        pending.push(row_id);
                        unique_results.push(row);
                    }
                }
                *row_page
                    .get_mut("results")
                    .context("Notion data-source query response is missing results")? =
                    serde_json::Value::Array(unique_results);
                search_pages.push(row_page);
            }
            source_objects.push(source);
        }
        data_sources.insert(database_id.clone(), source_objects);
        // Search may already have returned this database. Replace that
        // possibly sparse object with the full metadata response instead of
        // introducing a duplicate UUID into the inventory.
        let mut replaced = false;
        for page in &mut search_pages {
            if let Some(existing) = page
                .get_mut("results")
                .and_then(serde_json::Value::as_array_mut)
                .and_then(|results| {
                    results.iter_mut().find(|value| {
                        value.get("id").and_then(serde_json::Value::as_str)
                            == Some(database_id.as_str())
                            && object_kind(value).is_ok_and(|kind| kind == "database")
                    })
                })
            {
                *existing = database.clone();
                replaced = true;
                break;
            }
        }
        if !replaced {
            search_pages
                .push(serde_json::json!({"object":"list","results":[database],"has_more":false}));
        }
        // Rows can contain blocks and inline databases too. Search pages were
        // crawled above; query results enter the same fail-closed block walk.
        crawl_pending_blocks(
            api,
            &mut pending,
            &mut seen_parents,
            &mut seen_objects,
            &mut block_objects,
            &mut inline_database_ids,
        )?;
        database_ids.extend(inline_database_ids.iter().cloned());
    }
    if !block_objects.is_empty() {
        search_pages.push(serde_json::json!({
            "object": "list",
            "results": block_objects,
            "has_more": false
        }));
    }
    let mut availability = BTreeMap::new();
    for page_id in search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter(|value| object_kind(value).is_ok_and(|kind| kind == "page"))
        .filter_map(|value| value.get("id").and_then(|v| v.as_str()))
    {
        availability.insert(page_id.to_owned(), api.comments_available(page_id)?);
    }
    Inventory::from_recorded_responses(
        &search_pages,
        &data_sources,
        api.visible_share_ids(),
        availability,
        api.workspace_edition(),
    )
}

fn page_ids(pages: &[serde_json::Value]) -> Result<Vec<String>> {
    pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter_map(|value| match object_kind(value) {
            Ok(kind) if kind == "page" => Some(required_string(value, "id")),
            _ => None,
        })
        .collect()
}

fn crawl_pending_blocks(
    api: &mut impl NotionApi,
    pending: &mut Vec<String>,
    seen_parents: &mut std::collections::BTreeSet<String>,
    seen_objects: &mut std::collections::BTreeSet<String>,
    block_objects: &mut Vec<serde_json::Value>,
    inline_database_ids: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    while let Some(parent_id) = pending.pop() {
        if !seen_parents.insert(parent_id.clone()) {
            continue;
        }
        let pages = crawl_pages(|cursor| api.block_children(&parent_id, cursor))?;
        for page in pages {
            let results = page
                .get("results")
                .and_then(serde_json::Value::as_array)
                .context("Notion block-children response is missing results array")?;
            for value in results {
                let id = required_string(value, "id")?;
                if is_child_database_block(value) {
                    inline_database_ids.insert(id.clone());
                }
                if seen_objects.insert(id.clone()) {
                    // A child-database block's ID is the database ID itself;
                    // retain the database object fetched below, not a second
                    // inventory object with the same identity.
                    if !is_child_database_block(value) {
                        block_objects.push(value.clone());
                    }
                }
                if object_kind(value).is_ok_and(|kind| matches!(kind.as_str(), "block" | "page"))
                    && value.get("has_children").and_then(|v| v.as_bool()) == Some(true)
                {
                    pending.push(id);
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn crawl_pages(
    mut fetch: impl FnMut(Option<&str>) -> Result<serde_json::Value>,
) -> Result<Vec<serde_json::Value>> {
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        let mut page = fetch(cursor.as_deref())?;
        let has_more = page
            .get("has_more")
            .and_then(|v| v.as_bool())
            .context("Notion response is missing boolean has_more")?;
        let next = page
            .get("next_cursor")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        if has_more {
            let next = next.context("Notion response has_more=true without next_cursor")?;
            if !seen.insert(next.clone()) {
                bail!("Notion pagination cursor repeated: {next}");
            }
            cursor = Some(next);
            // The response is individually partial, but the concatenated
            // crawl is complete if all cursors resolve. Record that validated
            // fact for the downstream fixture parser.
            page["has_more"] = serde_json::Value::Bool(false);
        }
        pages.push(page);
        if !has_more {
            break;
        }
    }
    Ok(pages)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Inventory,
    Files,
    Content,
    References,
    Report,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Clean,
    Degraded,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InventoryObject {
    pub id: String,
    pub kind: String,
    pub parent_id: Option<String>,
    /// API object with Notion-hosted expiring file URLs removed. Later file
    /// acquisition must fetch the object again for a fresh URL; this inventory
    /// intentionally does not retain temporary download credentials.
    pub source: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Inventory {
    /// Page, database, and block objects own per-object progress checkpoints.
    pub objects: BTreeMap<String, InventoryObject>,
    /// Top-level page/database IDs visible to this integration.
    pub visible_shares: Vec<String>,
    /// Per-page `true` means the comments endpoint was successfully queried;
    /// it does not claim that the page has comments.
    pub comments_available: BTreeMap<String, bool>,
    /// Database ID -> its data-source IDs and complete source objects. The
    /// inventory is returned only after all listed sources were retrieved;
    /// slice 1 checkpoints them as one complete set under the database's
    /// `Stage::Inventory` progress row. There are no per-source progress rows.
    pub data_sources: BTreeMap<String, Vec<InventoryObject>>,
    pub workspace_edition: Option<String>,
}

impl Inventory {
    /// Build inventory from fully-paginated search, database metadata, and
    /// retrieved data-source objects. Database IDs discovered in search
    /// results (including data-source parents and child-database blocks) must
    /// have a recorded database and a source enumeration.
    pub fn from_recorded_responses(
        search_pages: &[serde_json::Value],
        data_sources_by_database: &BTreeMap<String, Vec<serde_json::Value>>,
        visible_shares: Vec<String>,
        comments_available: BTreeMap<String, bool>,
        workspace_edition: Option<String>,
    ) -> Result<Self> {
        let mut inventory = Self {
            visible_shares,
            comments_available,
            workspace_edition,
            ..Self::default()
        };
        for page in search_pages {
            ensure_complete_page(page)?;
            let results = page
                .get("results")
                .and_then(|v| v.as_array())
                .context("Notion response is missing results array")?;
            for value in results {
                let kind = object_kind(value)?;
                let id = required_string(value, "id")?;
                // Data sources have their own identity map and are
                // checkpointed through their containing database in slice 1.
                if kind == "data_source" {
                    continue;
                }
                let parent_id = value.get("parent").and_then(parent_id);
                let object = InventoryObject {
                    id: id.clone(),
                    kind: kind.clone(),
                    parent_id: parent_id.clone(),
                    source: sanitized_source(value.clone()),
                };
                if inventory.objects.insert(id.clone(), object).is_some() {
                    bail!("duplicate Notion object id in inventory: {id}");
                }
                if parent_id.is_none() && matches!(kind.as_str(), "page" | "database") {
                    inventory.visible_shares.push(id);
                }
            }
        }
        inventory.visible_shares.sort();
        inventory.visible_shares.dedup();

        let mut discovered_databases: std::collections::BTreeSet<String> = inventory
            .objects
            .values()
            .filter(|object| object.kind == "database")
            .map(|object| object.id.clone())
            .collect();
        for value in search_pages
            .iter()
            .flat_map(|page| page["results"].as_array().into_iter().flatten())
        {
            if object_kind(value)? == "data_source" {
                discovered_databases.insert(data_source_parent_database_id(value)?);
            }
        }
        if discovered_databases.iter().any(|id| {
            !inventory
                .objects
                .get(id)
                .is_some_and(|object| object.kind == "database")
        }) {
            bail!("search data source references a database missing from inventory");
        }
        if discovered_databases
            .iter()
            .any(|id| !data_sources_by_database.contains_key(id))
        {
            bail!("database inventory is missing its data-source enumeration");
        }
        if data_sources_by_database
            .keys()
            .any(|id| !discovered_databases.contains(id))
        {
            bail!("data-source enumeration references an undiscovered database");
        }
        let mut all_source_ids = std::collections::BTreeSet::new();
        for (database_id, source_objects) in data_sources_by_database {
            let database = &inventory.objects[database_id].source;
            let expected_source_ids: std::collections::BTreeSet<String> = database
                .get("data_sources")
                .and_then(serde_json::Value::as_array)
                .context("database inventory is missing data_sources array")?
                .iter()
                .map(|source| required_string(source, "id"))
                .collect::<Result<_>>()?;
            if expected_source_ids.is_empty() {
                bail!("database {database_id} has no data sources");
            }
            let mut sources = Vec::new();
            for value in source_objects {
                let id = required_string(value, "id")?;
                if object_kind(value)? != "data_source" {
                    bail!("database {database_id} returned a non-data-source object");
                }
                if data_source_parent_database_id(value)? != database_id.as_str() {
                    bail!("data source {id} does not belong to database {database_id}");
                }
                let source = InventoryObject {
                    id: id.clone(),
                    kind: "data_source".into(),
                    parent_id: Some(database_id.clone()),
                    source: sanitized_source(value.clone()),
                };
                if sources.iter().any(|item: &InventoryObject| item.id == id) {
                    bail!("duplicate data source id for database {database_id}: {id}");
                }
                if !all_source_ids.insert(id.clone()) {
                    bail!("duplicate data source id across databases: {id}");
                }
                sources.push(source);
            }
            let actual_source_ids: std::collections::BTreeSet<String> =
                sources.iter().map(|source| source.id.clone()).collect();
            if actual_source_ids != expected_source_ids {
                bail!("data-source enumeration does not match database {database_id} metadata");
            }
            sources.sort_by(|a, b| a.id.cmp(&b.id));
            inventory.data_sources.insert(database_id.clone(), sources);
        }
        for value in search_pages
            .iter()
            .flat_map(|page| page["results"].as_array().into_iter().flatten())
        {
            if object_kind(value)? == "data_source" {
                let id = required_string(value, "id")?;
                let database_id = data_source_parent_database_id(value)?;
                if !inventory
                    .data_sources
                    .get(&database_id)
                    .is_some_and(|sources| sources.iter().any(|source| source.id == id))
                {
                    bail!(
                        "searched data source {id} is absent from database {database_id} metadata"
                    );
                }
            }
        }
        for page_id in inventory.comments_available.keys() {
            if !inventory.objects.contains_key(page_id) {
                bail!("comments availability references unknown page {page_id}");
            }
            if inventory.objects[page_id].kind != "page" {
                bail!("comments availability is only valid for pages: {page_id}");
            }
        }
        Ok(inventory)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectProgress {
    pub stage: Stage,
    pub outcome: Outcome,
    #[serde(default)]
    pub reason: Option<String>,
}

/// One fetched source file's blob identity (design §4). The blob itself is
/// content-addressed by the model's `raw-blake3-256` profile: the lowercase
/// hex BLAKE3-256 digest of the exact bytes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileBlobRecord {
    pub blob_hash: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema_version: u32,
    pub workspace_id: String,
    pub acquisition_paths: Vec<String>,
    pub notion_version: String,
    pub edition: Option<String>,
    pub inventory: Inventory,
    /// Stable Notion page UUID -> destination note UUID.
    pub page_id_to_note_id: BTreeMap<String, String>,
    /// Stable Notion database UUID -> destination note UUID. Assigned by
    /// [`Manifest::assign_database_identities`]; absent from slice-1/2
    /// manifests, so it defaults empty when an older manifest is loaded.
    #[serde(default)]
    pub database_id_to_note_id: BTreeMap<String, String>,
    /// Stable Notion block UUID -> (destination note UUID, block identity).
    pub block_id_to_identity: BTreeMap<String, (String, String)>,
    /// Stable data-source UUID -> (current database UUID, source identity).
    pub data_source_id_to_identity: BTreeMap<String, (String, String)>,
    /// Source file occurrence -> fetched blob (design §4). The key is the
    /// UUID of the block carrying the file; duplicate bytes share one
    /// `blob_hash` across occurrences. Absent from slice-1 manifests, so it
    /// defaults empty when an older manifest is loaded.
    #[serde(default)]
    pub file_blobs: BTreeMap<String, FileBlobRecord>,
    /// Page UUID -> its comment claims (§2.4), recorded by
    /// [`comments::convert_comments`]. Absent from pre-slice-4
    /// manifests, so it defaults empty when an older manifest is loaded.
    #[serde(default)]
    pub comment_claims: BTreeMap<String, Vec<crate::comments::CommentClaim>>,
    /// Page UUID -> whether the acquisition path could see the page's
    /// comments (§2.4). A page present here has had its comments
    /// converted (possibly to zero claims); a page absent here has not.
    /// Absent from pre-slice-4 manifests, so it defaults empty.
    #[serde(default)]
    pub comments_coverage: BTreeMap<String, crate::comments::CommentsCoverage>,
    /// Row page UUID -> its row conversion record (§2.2), recorded by
    /// [`rows::convert_row`]. Absent from pre-slice-3-remainder
    /// manifests, so it defaults empty when an older manifest is loaded.
    #[serde(default)]
    pub row_records: BTreeMap<String, crate::rows::RowConversion>,
    /// Versioned per-data-source schema claims. Superseded claims remain as
    /// provenance when a source moves or changes schema (§6.1).
    #[serde(default)]
    pub schema_claim_history: Vec<crate::databases::SchemaClaimRecord>,
    /// Page UUID -> its degradation ledger (slice 4 design record §2.4,
    /// §2.6), recorded by [`content::convert_page`]. Absent from
    /// pre-slice-4 manifests, so it defaults empty when an older
    /// manifest is loaded.
    #[serde(default)]
    pub page_losses: BTreeMap<String, crate::content::LossLedger>,
    /// Export path -> how that export file matched the API inventory
    /// (design §2.5/§2.6). Every uncertain match is named here. Absent from
    /// pre-export manifests, so it defaults empty.
    #[serde(default)]
    pub export_matches: BTreeMap<String, crate::export::ExportMatch>,
    /// Which acquisition front ends fed this manifest (design §2.6).
    /// Additive: absent from older manifests, so it defaults to all false.
    #[serde(default)]
    pub acquisition: Acquisition,
    /// Degraded mode only (design §2.5): page UUID -> the weaker identity
    /// its `page_id_to_note_id` key carries. Every key of
    /// `page_id_to_note_id` that appears here is a degraded key. Absent
    /// from older manifests, so it defaults empty.
    #[serde(default)]
    pub degraded_identities: BTreeMap<String, crate::export_only::DegradedIdentity>,
    /// Page UUID -> why its comments were not imported, for pages whose
    /// `comments_coverage` is unavailable for a known reason (an
    /// export-only run records `export-path`). Defaults empty.
    #[serde(default)]
    pub comments_uncovered_reasons: BTreeMap<String, String>,
    /// Progress checkpoints exist for page, database, and block UUIDs in
    /// `inventory.objects`; a database checkpoint represents its complete
    /// embedded data-source set in `inventory.data_sources`.
    pub objects: BTreeMap<String, ObjectProgress>,
}

/// Acquisition header (design §2.6): which front ends supplied content and
/// whether the run fell back to export-only degraded mode.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Acquisition {
    pub api: bool,
    pub export: bool,
    pub degraded_mode: bool,
}

impl Manifest {
    /// Create a manifest with the Notion API version reported by its transport.
    /// Before resuming, callers must verify workspace/version identity and
    /// inventory reconciliation; [`Manifest::load`] only parses the file.
    pub fn new(
        workspace_id: String,
        acquisition_paths: Vec<String>,
        notion_version: String,
        inventory: Inventory,
    ) -> Result<Self> {
        if workspace_id.trim().is_empty() {
            bail!("workspace id must not be empty");
        }
        if notion_version != NOTION_API_VERSION {
            bail!(
                "Notion API version {notion_version} does not match required {NOTION_API_VERSION}"
            );
        }
        let data_source_id_to_identity = inventory
            .data_sources
            .iter()
            .flat_map(|(database_id, sources)| {
                sources.iter().map(move |source| {
                    (source.id.clone(), (database_id.clone(), source.id.clone()))
                })
            })
            .collect();
        let acquisition = Acquisition {
            api: acquisition_paths.iter().any(|path| path == "api"),
            export: acquisition_paths.iter().any(|path| path == "export"),
            degraded_mode: false,
        };
        Ok(Self {
            schema_version: 1,
            workspace_id,
            acquisition_paths,
            notion_version,
            edition: inventory.workspace_edition.clone(),
            objects: inventory
                .objects
                .keys()
                .map(|id| {
                    (
                        id.clone(),
                        ObjectProgress {
                            stage: Stage::Inventory,
                            outcome: Outcome::Clean,
                            reason: None,
                        },
                    )
                })
                .collect(),
            inventory,
            page_id_to_note_id: BTreeMap::new(),
            database_id_to_note_id: BTreeMap::new(),
            block_id_to_identity: BTreeMap::new(),
            data_source_id_to_identity,
            file_blobs: BTreeMap::new(),
            comment_claims: BTreeMap::new(),
            comments_coverage: BTreeMap::new(),
            row_records: BTreeMap::new(),
            schema_claim_history: Vec::new(),
            page_losses: BTreeMap::new(),
            export_matches: BTreeMap::new(),
            acquisition,
            degraded_identities: BTreeMap::new(),
            comments_uncovered_reasons: BTreeMap::new(),
        })
    }

    /// Assign a destination note ID to every inventoried page that lacks
    /// one (design §3: identity is assigned for the whole inventory before
    /// any content converts, so forward references resolve). Assignments
    /// already recorded are never changed — that stability is what makes
    /// re-import idempotent (§6.1) — and a minter that returns an empty or
    /// already-used ID fails the pass rather than forking identity.
    /// Returns how many pages were newly assigned.
    pub fn assign_page_identities(
        &mut self,
        mint: &mut dyn FnMut(&str) -> Result<String>,
    ) -> Result<usize> {
        let page_ids: Vec<String> = self
            .inventory
            .objects
            .values()
            .filter(|object| object.kind == "page")
            .map(|object| object.id.clone())
            .collect();
        let mut taken: std::collections::BTreeSet<String> = self
            .page_id_to_note_id
            .values()
            .cloned()
            .chain(self.database_id_to_note_id.values().cloned())
            .collect();
        let mut assigned = 0;
        for page_id in page_ids {
            if self.page_id_to_note_id.contains_key(&page_id) {
                continue;
            }
            let note_id =
                mint(&page_id).with_context(|| format!("mint note id for page {page_id}"))?;
            if note_id.trim().is_empty() {
                bail!("note id minter returned an empty id for page {page_id}");
            }
            if !taken.insert(note_id.clone()) {
                bail!("note id minter returned duplicate id {note_id}");
            }
            self.page_id_to_note_id.insert(page_id, note_id);
            assigned += 1;
        }
        Ok(assigned)
    }

    /// Assign a destination note ID to every inventoried database that
    /// lacks one (design §2.2: one note per database; §3: identity is
    /// assigned for the whole inventory before any content converts, so
    /// forward references resolve). Mirrors
    /// [`Manifest::assign_page_identities`]: assignments already recorded
    /// are never changed, and a minter that returns an empty or
    /// already-used ID fails the pass rather than forking identity.
    /// Returns how many databases were newly assigned.
    pub fn assign_database_identities(
        &mut self,
        mint: &mut dyn FnMut(&str) -> Result<String>,
    ) -> Result<usize> {
        let database_ids: Vec<String> = self
            .inventory
            .objects
            .values()
            .filter(|object| object.kind == "database")
            .map(|object| object.id.clone())
            .collect();
        let mut taken: std::collections::BTreeSet<String> = self
            .database_id_to_note_id
            .values()
            .cloned()
            .chain(self.page_id_to_note_id.values().cloned())
            .collect();
        let mut assigned = 0;
        for database_id in database_ids {
            if self.database_id_to_note_id.contains_key(&database_id) {
                continue;
            }
            let note_id = mint(&database_id)
                .with_context(|| format!("mint note id for database {database_id}"))?;
            if note_id.trim().is_empty() {
                bail!("note id minter returned an empty id for database {database_id}");
            }
            if !taken.insert(note_id.clone()) {
                bail!("note id minter returned duplicate id {note_id}");
            }
            self.database_id_to_note_id.insert(database_id, note_id);
            assigned += 1;
        }
        Ok(assigned)
    }

    /// Reject resume against another workspace before callers create objects.
    pub fn verify_resume_identity(&self, workspace_id: &str, notion_version: &str) -> Result<()> {
        if self.workspace_id != workspace_id {
            bail!(
                "Notion workspace identity changed (manifest {}, current {})",
                self.workspace_id,
                workspace_id
            );
        }
        if self.notion_version != notion_version || notion_version != NOTION_API_VERSION {
            bail!(
                "Notion API version changed (manifest {}, current {})",
                self.notion_version,
                notion_version
            );
        }
        Ok(())
    }

    /// Persist per-object progress as work advances. Stages cannot move
    /// backwards; repeating the current stage is idempotent for retries.
    pub fn record_progress(
        &mut self,
        object_id: &str,
        stage: Stage,
        outcome: Outcome,
        reason: Option<String>,
    ) -> Result<()> {
        let progress = self
            .objects
            .get_mut(object_id)
            .with_context(|| format!("object {object_id} is not in the manifest inventory"))?;
        let rank = |value: &Stage| match value {
            Stage::Inventory => 0,
            Stage::Files => 1,
            Stage::Content => 2,
            Stage::References => 3,
            Stage::Report => 4,
        };
        if rank(&stage) < rank(&progress.stage) {
            bail!("manifest stage cannot move backwards for {object_id}");
        }
        progress.stage = stage;
        progress.outcome = outcome;
        progress.reason = reason;
        Ok(())
    }

    /// The inventory accounting invariant is checked before any later import
    /// stage may create objects.
    pub fn validate_inventory_reconciles(&self) -> Result<()> {
        if self.objects.len() != self.inventory.objects.len()
            || self
                .objects
                .keys()
                .any(|id| !self.inventory.objects.contains_key(id))
        {
            bail!("manifest progress does not reconcile to inventory objects");
        }
        Ok(())
    }

    /// Write by rename so a crash cannot leave a truncated manifest. No token
    /// field exists in the schema; callers should not put credentials in IDs,
    /// paths or reason strings.
    pub fn save_atomic(&self, path: &Path) -> Result<()> {
        let parent = manifest_parent(path);
        fs::create_dir_all(parent)?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).context("create unique manifest temp file")?;
        serde_json::to_writer_pretty(temp.as_file_mut(), self).context("write manifest temp")?;
        temp.as_file().sync_all().context("sync manifest temp")?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("replace manifest {}", path.display()))?;
        #[cfg(unix)]
        File::open(parent)
            .context("open manifest directory for sync")?
            .sync_all()
            .context("sync manifest directory")?;
        Ok(())
    }

    /// Parse a manifest and validate its schema version. Before resuming,
    /// callers must additionally call [`Manifest::verify_resume_identity`] and
    /// [`Manifest::validate_inventory_reconciles`].
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read manifest {}", path.display()))?;
        let manifest: Self =
            serde_json::from_slice(&bytes).context("parse Notion import manifest")?;
        if manifest.schema_version != 1 {
            bail!(
                "unsupported manifest schema version {}",
                manifest.schema_version
            );
        }
        // Parsing is not resume validation: callers must invoke
        // `verify_resume_identity` and `validate_inventory_reconciles`.
        Ok(manifest)
    }
}

fn manifest_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn ensure_complete_page(page: &serde_json::Value) -> Result<()> {
    if page
        .get("has_more")
        .and_then(|v| v.as_bool())
        .context("Notion response is missing boolean has_more")?
    {
        bail!("recorded Notion response has_more=true; inventory pagination is incomplete");
    }
    Ok(())
}

pub(crate) fn required_string(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .with_context(|| format!("Notion object is missing string field {key}"))
}

fn data_source_parent_database_id(value: &serde_json::Value) -> Result<String> {
    let parent = value
        .get("parent")
        .context("Notion data source is missing parent")?;
    if parent.get("type").and_then(serde_json::Value::as_str) != Some("database_id") {
        bail!("Notion data source parent is not a database");
    }
    required_string(parent, "database_id")
}

fn object_kind(value: &serde_json::Value) -> Result<String> {
    if value.get("type").and_then(serde_json::Value::as_str) == Some("child_page") {
        return Ok("page".into());
    }
    if let Some(object) = value.get("object").and_then(serde_json::Value::as_str) {
        if matches!(object, "page" | "database" | "data_source" | "block") {
            return Ok(object.into());
        }
    }
    for kind in ["page", "database", "data_source", "block"] {
        if value.get(kind).is_some() {
            return Ok(kind.into());
        }
    }
    bail!("unsupported Notion inventory object kind")
}

fn is_child_database_block(value: &serde_json::Value) -> bool {
    object_kind(value).is_ok_and(|kind| kind == "block")
        && value.get("type").and_then(serde_json::Value::as_str) == Some("child_database")
}

fn parent_id(parent: &serde_json::Value) -> Option<String> {
    let key = match parent.get("type")?.as_str()? {
        "page_id" => "page_id",
        "database_id" => "database_id",
        "block_id" => "block_id",
        "data_source_id" => "data_source_id",
        _ => return None,
    };
    parent.get(key)?.as_str().map(str::to_owned)
}

pub(crate) fn sanitized_source(mut value: serde_json::Value) -> serde_json::Value {
    fn redact(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("type").and_then(serde_json::Value::as_str) == Some("file")
                    && let Some(serde_json::Value::Object(file)) = map.get_mut("file")
                {
                    file.remove("url");
                    file.remove("expiry_time");
                }
                for child in map.values_mut() {
                    redact(child);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(redact),
            _ => {}
        }
    }
    redact(&mut value);
    value
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(results: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"object":"list", "results":results, "has_more":false})
    }

    #[test]
    fn bare_manifest_filename_uses_current_directory() {
        assert_eq!(manifest_parent(Path::new("manifest.json")), Path::new("."));
    }

    struct FixtureApi {
        search_calls: usize,
        version: &'static str,
    }
    impl NotionApi for FixtureApi {
        fn notion_version(&self) -> &str {
            self.version
        }

        fn search(&mut self, cursor: Option<&str>) -> Result<serde_json::Value> {
            self.search_calls += 1;
            Ok(match cursor {
                None => {
                    json!({"results":[{"object":"data_source","id":"ds-paged","parent":{"type":"database_id","database_id":"db-paged"}}],"has_more":true,"next_cursor":"next"})
                }
                Some("next") => page(vec![
                    json!({"object":"database","id":"db-paged","parent":{"type":"workspace","workspace":true}}),
                    json!({"object":"page","id":"page-paged","parent":{"type":"workspace"}}),
                ]),
                _ => bail!("unexpected cursor"),
            })
        }
        fn block_children(
            &mut self,
            block_id: &str,
            _cursor: Option<&str>,
        ) -> Result<serde_json::Value> {
            match block_id {
                "missing-results" => Ok(json!({"has_more":false})),
                "page-paged" => Ok(page(vec![json!({
                    "object":"block",
                    "id":"block-parent",
                    "has_children":true,
                    "parent":{"type":"page_id","page_id":"page-paged"}
                })])),
                "row-paged" => Ok(page(vec![json!({
                    "object":"block",
                    "id":"row-block",
                    "has_children":false,
                    "parent":{"type":"page_id","page_id":"row-paged"}
                })])),
                "row-paged-2" => Ok(page(vec![])),
                "block-parent" => Ok(page(vec![
                    json!({
                        "object":"block",
                        "id":"block-child",
                        "type":"image",
                        "image":{"type":"file","file":{"url":"https://files.notion.so/signed-block-secret?token=fixture-secret","expiry_time":"2026-11-21T00:00:00Z"}},
                        "has_children":false,
                        "parent":{"type":"block_id","block_id":"block-parent"}
                    }),
                    json!({
                        "object":"block",
                        "id":"page-child",
                        "type":"child_page",
                        "child_page":{"title":"Nested"},
                        "has_children":false,
                        "parent":{"type":"block_id","block_id":"block-parent"}
                    }),
                    json!({
                        "object":"block",
                        "id":"db-inline",
                        "type":"child_database",
                        "child_database":{"title":"Inline"},
                        "has_children":false,
                        "parent":{"type":"page_id","page_id":"page-paged"}
                    }),
                ])),
                _ => bail!("unexpected block parent {block_id}"),
            }
        }
        fn database(&mut self, database_id: &str) -> Result<serde_json::Value> {
            let source_id = if database_id == "db-inline" {
                "ds-inline"
            } else {
                "ds-paged"
            };
            Ok(
                json!({"object":"database","id":database_id,"parent":{"type":"workspace","workspace":true},"data_sources":[{"id":source_id,"name":"Tasks"}]}),
            )
        }
        fn data_source(&mut self, data_source_id: &str) -> Result<serde_json::Value> {
            let database_id = if data_source_id == "ds-inline" {
                "db-inline"
            } else {
                "db-paged"
            };
            Ok(
                json!({"object":"data_source","id":data_source_id,"parent":{"type":"database_id","database_id":database_id},"properties":{"Name":{"type":"title"}}}),
            )
        }
        fn query_data_source(
            &mut self,
            data_source_id: &str,
            cursor: Option<&str>,
        ) -> Result<serde_json::Value> {
            if data_source_id != "ds-paged" {
                return Ok(page(vec![]));
            }
            match cursor {
                None => Ok(json!({
                    "results":[{"object":"page","id":"row-paged","parent":{"type":"data_source_id","data_source_id":"ds-paged","database_id":"db-paged"}}],
                    "has_more":true,
                    "next_cursor":"row-next"
                })),
                Some("row-next") => Ok(page(vec![json!({
                    "object":"page",
                    "id":"row-paged-2",
                    "parent":{"type":"data_source_id","data_source_id":"ds-paged","database_id":"db-paged"}
                })])),
                _ => bail!("unexpected data-source cursor"),
            }
        }
        fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
            Ok(true)
        }
        fn visible_share_ids(&self) -> Vec<String> {
            vec!["db-paged".into(), "page-paged".into(), "db-inline".into()]
        }
        fn workspace_edition(&self) -> Option<String> {
            Some("business".into())
        }
    }

    #[test]
    fn api_crawl_exhausts_cursors_before_returning() {
        let mut api = FixtureApi {
            search_calls: 0,
            version: NOTION_API_VERSION,
        };
        let inventory = crawl_inventory(&mut api).unwrap();
        assert_eq!(api.search_calls, 2);
        assert_eq!(inventory.objects.len(), 9);
        assert_eq!(inventory.data_sources["db-paged"][0].id, "ds-paged");
        assert_eq!(inventory.data_sources["db-inline"][0].id, "ds-inline");
        assert_eq!(inventory.comments_available["page-paged"], true);
        assert_eq!(inventory.comments_available["page-child"], true);
        assert_eq!(inventory.objects["page-child"].kind, "page");
        assert!(inventory.objects.contains_key("row-paged"));
        assert!(inventory.objects.contains_key("row-paged-2"));
        assert_eq!(
            inventory.objects["row-block"].parent_id.as_deref(),
            Some("row-paged")
        );
        assert_eq!(
            inventory.objects["block-parent"].parent_id.as_deref(),
            Some("page-paged")
        );
        assert_eq!(
            inventory.objects["block-child"].parent_id.as_deref(),
            Some("block-parent")
        );
        assert!(
            inventory.objects["block-child"].source["image"]["file"]
                .get("url")
                .is_none()
        );
    }

    /// A wiki whose query also lists a child data source, and an optional
    /// capped query.
    struct WikiApi {
        capped: bool,
    }
    impl NotionApi for WikiApi {
        fn notion_version(&self) -> &str {
            NOTION_API_VERSION
        }
        fn search(&mut self, _cursor: Option<&str>) -> Result<serde_json::Value> {
            Ok(page(vec![json!({
                "object":"database","id":"db-wiki","parent":{"type":"workspace","workspace":true}
            })]))
        }
        fn block_children(
            &mut self,
            _block_id: &str,
            _cursor: Option<&str>,
        ) -> Result<serde_json::Value> {
            Ok(page(vec![]))
        }
        fn database(&mut self, database_id: &str) -> Result<serde_json::Value> {
            let source = if database_id == "db-wiki" {
                "ds-wiki"
            } else {
                "ds-child"
            };
            Ok(
                json!({"object":"database","id":database_id,"parent":{"type":"workspace","workspace":true},"data_sources":[{"id":source,"name":"S"}]}),
            )
        }
        fn data_source(&mut self, data_source_id: &str) -> Result<serde_json::Value> {
            let database = if data_source_id == "ds-wiki" {
                "db-wiki"
            } else {
                "db-child"
            };
            Ok(
                json!({"object":"data_source","id":data_source_id,"parent":{"type":"database_id","database_id":database},"properties":{"Name":{"type":"title"}}}),
            )
        }
        fn query_data_source(
            &mut self,
            data_source_id: &str,
            _cursor: Option<&str>,
        ) -> Result<serde_json::Value> {
            Ok(if data_source_id == "ds-wiki" {
                let mut response = page(vec![
                    json!({"object":"page","id":"row-wiki","parent":{"type":"data_source_id","data_source_id":"ds-wiki","database_id":"db-wiki"}}),
                    json!({"object":"data_source","id":"ds-child","parent":{"type":"database_id","database_id":"db-child"}}),
                ]);
                if self.capped {
                    response["request_status"] = json!({"type":"incomplete"});
                }
                response
            } else {
                page(vec![
                    json!({"object":"page","id":"row-child","parent":{"type":"data_source_id","data_source_id":"ds-child","database_id":"db-child"}}),
                ])
            })
        }
        fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
            Ok(true)
        }
        fn visible_share_ids(&self) -> Vec<String> {
            vec!["db-wiki".into()]
        }
        fn workspace_edition(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn wiki_query_data_sources_are_enumerated_not_rejected() {
        let inventory = crawl_inventory(&mut WikiApi { capped: false }).unwrap();
        assert_eq!(inventory.data_sources["db-child"][0].id, "ds-child");
        assert!(inventory.objects.contains_key("row-wiki"));
        assert!(inventory.objects.contains_key("row-child"));
    }

    #[test]
    fn capped_data_source_query_fails_closed() {
        let err = crawl_inventory(&mut WikiApi { capped: true }).unwrap_err();
        assert!(err.to_string().contains("incomplete"), "{err:#}");
    }

    #[test]
    fn block_children_response_without_results_fails_closed() {
        let mut api = FixtureApi {
            search_calls: 0,
            version: NOTION_API_VERSION,
        };
        let mut pending = vec!["missing-results".to_owned()];
        assert!(
            crawl_pending_blocks(
                &mut api,
                &mut pending,
                &mut std::collections::BTreeSet::new(),
                &mut std::collections::BTreeSet::new(),
                &mut Vec::new(),
                &mut std::collections::BTreeSet::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn recorded_api_fixture_produces_complete_inventory_and_manifest() {
        let database = json!({"object":"database", "id":"db-1", "parent":{"type":"workspace","workspace":true}, "title":[{"plain_text":"Tasks"}], "data_sources":[{"id":"source-1"}]});
        let item = json!({"object":"page", "id":"page-1", "parent":{"type":"database_id","database_id":"db-1"}});
        let schema = json!({"object":"data_source", "id":"source-1", "parent":{"type":"database_id","database_id":"db-1"}, "properties":{"Name":{"type":"title"}}});
        let mut sources = BTreeMap::new();
        sources.insert("db-1".into(), vec![schema]);
        let inventory = Inventory::from_recorded_responses(
            &[page(vec![database, item])],
            &sources,
            vec!["db-1".into()],
            BTreeMap::from([("page-1".into(), true)]),
            Some("business".into()),
        )
        .unwrap();
        assert_eq!(inventory.objects.len(), 2);
        assert_eq!(inventory.data_sources["db-1"][0].id, "source-1");
        assert_eq!(inventory.comments_available["page-1"], true);
        let manifest = Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
            NOTION_API_VERSION.into(),
            inventory,
        )
        .unwrap();
        assert_eq!(manifest.notion_version, NOTION_API_VERSION);
        assert_eq!(manifest.objects.len(), 2);
    }

    #[test]
    fn pagination_and_workspace_change_fail_closed() {
        let partial = json!({"results":[], "has_more":true});
        assert!(
            Inventory::from_recorded_responses(
                &[partial],
                &BTreeMap::new(),
                vec![],
                BTreeMap::new(),
                None
            )
            .is_err()
        );
        let manifest = Manifest::new(
            "workspace-1".into(),
            vec![],
            NOTION_API_VERSION.into(),
            Inventory::default(),
        )
        .unwrap();
        assert!(
            manifest
                .verify_resume_identity("workspace-2", NOTION_API_VERSION)
                .is_err()
        );
        assert!(
            manifest
                .verify_resume_identity("workspace-1", "2022-06-28")
                .is_err()
        );
        assert!(crawl_pages(|_| Ok(json!({"results":[]}))).is_err());
        assert!(crawl_pages(|_| Ok(json!({"results":[],"has_more":"true"}))).is_err());
        assert!(crawl_pages(|_| Ok(json!({"results":[],"has_more":true}))).is_err());
        let mut cursors = Vec::new();
        let pages = crawl_pages(|cursor| {
            cursors.push(cursor.map(str::to_owned));
            match cursor {
                None => Ok(json!({"results":[],"has_more":true,"next_cursor":"empty-next"})),
                Some("empty-next") => Ok(page(vec![])),
                _ => bail!("unexpected cursor"),
            }
        })
        .unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(cursors, [None, Some("empty-next".into())]);
    }

    #[test]
    fn crawl_rejects_transport_version_mismatch_before_requests() {
        let mut api = FixtureApi {
            search_calls: 0,
            version: "2022-06-28",
        };
        assert!(crawl_inventory(&mut api).is_err());
        assert_eq!(api.search_calls, 0);
    }

    #[test]
    fn parent_and_object_kind_follow_notion_types() {
        assert_eq!(
            object_kind(&json!({"object":"block","type":"child_page"})).unwrap(),
            "page"
        );
        assert_eq!(
            object_kind(&json!({"type":"child_page","child_page":{"title":"Nested"}})).unwrap(),
            "page"
        );
        assert_eq!(
            parent_id(
                &json!({"type":"data_source_id","data_source_id":"ds-1","database_id":"db-1"})
            ),
            Some("ds-1".into())
        );
        assert_eq!(
            parent_id(&json!({"type":"workspace","workspace":true})),
            None
        );
    }

    #[test]
    fn manifest_round_trips_without_credential_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let manifest = Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
            NOTION_API_VERSION.into(),
            Inventory::default(),
        )
        .unwrap();
        manifest.save_atomic(&path).unwrap();
        let encoded = fs::read_to_string(&path).unwrap();
        assert!(!encoded.contains("token"));
        assert_eq!(Manifest::load(&path).unwrap(), manifest);
    }

    #[test]
    fn progress_is_resumable_and_cannot_rewind() {
        let inventory = Inventory::from_recorded_responses(
            &[page(vec![json!({"object":"page","id":"p1"})])],
            &BTreeMap::new(),
            vec!["p1".into()],
            BTreeMap::from([("p1".into(), false)]),
            None,
        )
        .unwrap();
        let mut manifest = Manifest::new(
            "w1".into(),
            vec!["api".into()],
            NOTION_API_VERSION.into(),
            inventory,
        )
        .unwrap();
        manifest.validate_inventory_reconciles().unwrap();
        manifest
            .record_progress("p1", Stage::Content, Outcome::Clean, None)
            .unwrap();
        assert!(
            manifest
                .record_progress("p1", Stage::Files, Outcome::Clean, None)
                .is_err()
        );
    }
}
