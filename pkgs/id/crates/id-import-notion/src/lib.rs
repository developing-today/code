//! Durable inventory and resume primitives for the Notion importer, slice 1.
//!
//! The API transport is deliberately injected by callers. In particular, a
//! bearer token is never accepted by or serialized from [`Manifest`].

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const NOTION_API_VERSION: &str = "2025-09-03";

/// Minimal authenticated transport boundary. Implementations receive the
/// credential from the caller and must not retain or log it. The acquisition
/// workflow owns rate limiting/retries; each method returns one raw API page.
pub trait NotionApi {
    fn search(&mut self, start_cursor: Option<&str>) -> Result<serde_json::Value>;
    fn block_children(
        &mut self,
        block_id: &str,
        start_cursor: Option<&str>,
    ) -> Result<serde_json::Value>;
    fn data_sources(
        &mut self,
        database_id: &str,
        start_cursor: Option<&str>,
    ) -> Result<serde_json::Value>;
    fn comments_available(&mut self, page_id: &str) -> Result<bool>;
    fn visible_share_ids(&self) -> Vec<String>;
    fn workspace_edition(&self) -> Option<String>;
}

/// Crawl inventory endpoints completely before returning. This function does
/// not create ID objects; it is safe to use as the mandatory inventory-first
/// preflight. An API failure or pagination cycle returns an error, never a
/// partial inventory.
pub fn crawl_inventory(api: &mut impl NotionApi) -> Result<Inventory> {
    let mut search_pages = crawl_pages(|cursor| api.search(cursor))?;
    let mut pending: Vec<String> = search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter(|value| {
            matches!(object_kind(value).as_deref(), Ok("page" | "database"))
                && value.get("has_children").and_then(|v| v.as_bool()) == Some(true)
        })
        .filter_map(|value| value.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let mut seen_parents = std::collections::BTreeSet::new();
    let mut seen_objects: std::collections::BTreeSet<String> = search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter_map(|value| value.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let mut block_objects = Vec::new();
    while let Some(parent_id) = pending.pop() {
        if !seen_parents.insert(parent_id.clone()) {
            continue;
        }
        let pages = crawl_pages(|cursor| api.block_children(&parent_id, cursor))?;
        for page in pages {
            for value in page["results"].as_array().into_iter().flatten() {
                let id = required_string(value, "id")?;
                if seen_objects.insert(id.clone()) {
                    block_objects.push(value.clone());
                }
                if object_kind(value).is_ok_and(|kind| kind == "block")
                    && value.get("has_children").and_then(|v| v.as_bool()) == Some(true)
                {
                    pending.push(id);
                }
            }
        }
    }
    if !block_objects.is_empty() {
        search_pages.push(serde_json::json!({
            "object": "list",
            "results": block_objects,
            "has_more": false
        }));
    }
    let database_ids: Vec<String> = search_pages
        .iter()
        .flat_map(|page| page["results"].as_array().into_iter().flatten())
        .filter(|value| object_kind(value).is_ok_and(|kind| kind == "database"))
        .filter_map(|value| value.get("id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let mut data_sources = BTreeMap::new();
    for database_id in database_ids {
        data_sources.insert(
            database_id.clone(),
            crawl_pages(|cursor| api.data_sources(&database_id, cursor))?,
        );
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

fn crawl_pages(
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
            .unwrap_or(false);
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
    /// Complete API object is retained so schema and source facts are not
    /// silently discarded by this inventory-only slice.
    pub source: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Inventory {
    pub objects: BTreeMap<String, InventoryObject>,
    /// Top-level page/database IDs visible to this integration.
    pub visible_shares: Vec<String>,
    /// Per-page `true` means the comments endpoint was successfully queried;
    /// it does not claim that the page has comments.
    pub comments_available: BTreeMap<String, bool>,
    /// Database ID -> its data-source IDs and complete source objects.
    pub data_sources: BTreeMap<String, Vec<InventoryObject>>,
    pub workspace_edition: Option<String>,
}

impl Inventory {
    /// Build inventory from recorded `search` and `data_sources` API response
    /// pages. Pagination must be exhausted by the acquisition layer; the
    /// function rejects a response marked `has_more` to prevent partial
    /// inventories being mistaken for complete ones.
    pub fn from_recorded_responses(
        search_pages: &[serde_json::Value],
        data_source_pages: &BTreeMap<String, Vec<serde_json::Value>>,
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
                let id = required_string(value, "id")?;
                let kind = object_kind(value)?;
                let parent_id = value.get("parent").and_then(parent_id);
                let object = InventoryObject {
                    id: id.clone(),
                    kind: kind.clone(),
                    parent_id: parent_id.clone(),
                    source: value.clone(),
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

        for (database_id, pages) in data_source_pages {
            let mut sources = Vec::new();
            for page in pages {
                ensure_complete_page(page)?;
                let results = page
                    .get("results")
                    .and_then(|v| v.as_array())
                    .context("data-source response is missing results array")?;
                for value in results {
                    let id = required_string(value, "id")?;
                    let source = InventoryObject {
                        id: id.clone(),
                        kind: "data_source".into(),
                        parent_id: Some(database_id.clone()),
                        source: value.clone(),
                    };
                    if sources.iter().any(|item: &InventoryObject| item.id == id) {
                        bail!("duplicate data source id for database {database_id}: {id}");
                    }
                    sources.push(source);
                }
            }
            sources.sort_by(|a, b| a.id.cmp(&b.id));
            inventory.data_sources.insert(database_id.clone(), sources);
        }
        for (page_id, available) in &inventory.comments_available {
            if !inventory.objects.contains_key(page_id) {
                bail!("comments availability references unknown page {page_id}");
            }
            if inventory.objects[page_id].kind != "page" {
                bail!("comments availability is only valid for pages: {page_id}");
            }
            let _ = available;
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema_version: u32,
    pub workspace_id: String,
    pub acquisition_paths: Vec<String>,
    pub notion_version: String,
    pub edition: Option<String>,
    pub inventory: Inventory,
    pub page_id_to_note_id: BTreeMap<String, String>,
    pub block_id_to_identity: BTreeMap<String, (String, String)>,
    pub data_source_id_to_identity: BTreeMap<String, (String, String)>,
    pub objects: BTreeMap<String, ObjectProgress>,
}

impl Manifest {
    pub fn new(
        workspace_id: String,
        acquisition_paths: Vec<String>,
        inventory: Inventory,
    ) -> Result<Self> {
        if workspace_id.trim().is_empty() {
            bail!("workspace id must not be empty");
        }
        Ok(Self {
            schema_version: 1,
            workspace_id,
            acquisition_paths,
            notion_version: NOTION_API_VERSION.into(),
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
            block_id_to_identity: BTreeMap::new(),
            data_source_id_to_identity: BTreeMap::new(),
        })
    }

    /// Reject resume against another workspace before callers create objects.
    pub fn verify_resume_identity(&self, workspace_id: &str) -> Result<()> {
        if self.workspace_id != workspace_id {
            bail!(
                "Notion workspace identity changed (manifest {}, current {})",
                self.workspace_id,
                workspace_id
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
        let bytes = serde_json::to_vec_pretty(self)?;
        let parent = path
            .parent()
            .context("manifest path has no parent directory")?;
        fs::create_dir_all(parent)?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes)
            .with_context(|| format!("write manifest temp {}", temp.display()))?;
        fs::rename(&temp, path).with_context(|| format!("replace manifest {}", path.display()))?;
        Ok(())
    }

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
        Ok(manifest)
    }
}

fn ensure_complete_page(page: &serde_json::Value) -> Result<()> {
    if page
        .get("has_more")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        bail!("recorded Notion response has_more=true; inventory pagination is incomplete");
    }
    Ok(())
}

fn required_string(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .with_context(|| format!("Notion object is missing string field {key}"))
}

fn object_kind(value: &serde_json::Value) -> Result<String> {
    for kind in ["page", "database", "data_source", "block"] {
        if value.get(kind).is_some() || value.get("object").and_then(|v| v.as_str()) == Some(kind) {
            return Ok(kind.into());
        }
    }
    bail!("unsupported Notion inventory object kind")
}

fn parent_id(parent: &serde_json::Value) -> Option<String> {
    ["page_id", "database_id", "block_id", "workspace_id"]
        .iter()
        .find_map(|key| parent.get(key).and_then(|v| v.as_str()).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(results: Vec<serde_json::Value>) -> serde_json::Value {
        json!({"object":"list", "results":results, "has_more":false})
    }

    struct FixtureApi {
        search_calls: usize,
    }
    impl NotionApi for FixtureApi {
        fn search(&mut self, cursor: Option<&str>) -> Result<serde_json::Value> {
            self.search_calls += 1;
            Ok(match cursor {
                None => {
                    json!({"results":[{"object":"database","id":"db-paged","parent":{"type":"workspace"}}],"has_more":true,"next_cursor":"next"})
                }
                Some("next") => page(vec![
                    json!({"object":"page","id":"page-paged","has_children":true,"parent":{"type":"workspace"}}),
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
                "page-paged" => Ok(page(vec![json!({
                    "object":"block",
                    "id":"block-parent",
                    "has_children":true,
                    "parent":{"type":"page_id","page_id":"page-paged"}
                })])),
                "block-parent" => Ok(page(vec![json!({
                    "object":"block",
                    "id":"block-child",
                    "has_children":false,
                    "parent":{"type":"block_id","block_id":"block-parent"}
                })])),
                _ => bail!("unexpected block parent {block_id}"),
            }
        }
        fn data_sources(
            &mut self,
            _database_id: &str,
            _cursor: Option<&str>,
        ) -> Result<serde_json::Value> {
            Ok(page(vec![
                json!({"object":"data_source","id":"ds-paged","properties":{}}),
            ]))
        }
        fn comments_available(&mut self, _page_id: &str) -> Result<bool> {
            Ok(true)
        }
        fn visible_share_ids(&self) -> Vec<String> {
            vec!["db-paged".into(), "page-paged".into()]
        }
        fn workspace_edition(&self) -> Option<String> {
            Some("business".into())
        }
    }

    #[test]
    fn api_crawl_exhausts_cursors_before_returning() {
        let mut api = FixtureApi { search_calls: 0 };
        let inventory = crawl_inventory(&mut api).unwrap();
        assert_eq!(api.search_calls, 2);
        assert_eq!(inventory.objects.len(), 4);
        assert_eq!(inventory.data_sources["db-paged"][0].id, "ds-paged");
        assert_eq!(inventory.comments_available["page-paged"], true);
        assert_eq!(inventory.objects["block-parent"].parent_id.as_deref(), Some("page-paged"));
        assert_eq!(inventory.objects["block-child"].parent_id.as_deref(), Some("block-parent"));
    }

    #[test]
    fn recorded_api_fixture_produces_complete_inventory_and_manifest() {
        let database = json!({"object":"database", "id":"db-1", "parent":{"type":"workspace","workspace":true}, "title":[{"plain_text":"Tasks"}]});
        let item = json!({"object":"page", "id":"page-1", "parent":{"type":"database_id","database_id":"db-1"}});
        let schema = json!({"object":"data_source", "id":"source-1", "properties":{"Name":{"type":"title"}}});
        let mut sources = BTreeMap::new();
        sources.insert("db-1".into(), vec![page(vec![schema])]);
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
        let manifest = Manifest::new("workspace-1".into(), vec!["api".into()], inventory).unwrap();
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
        let manifest = Manifest::new("workspace-1".into(), vec![], Inventory::default()).unwrap();
        assert!(manifest.verify_resume_identity("workspace-2").is_err());
    }

    #[test]
    fn manifest_round_trips_without_credential_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let manifest = Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
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
        let mut manifest = Manifest::new("w1".into(), vec!["api".into()], inventory).unwrap();
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
