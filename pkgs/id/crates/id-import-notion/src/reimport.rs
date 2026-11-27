//! Deterministic inventory diffing and move reconciliation for re-imports (§6.1–§6.2).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Inventory, InventoryObject, Manifest, databases::convert_database};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    New,
    Changed,
    Unchanged,
    MissingFromSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InventoryChange {
    pub object_id: String,
    pub object_kind: String,
    pub parent_id: Option<String>,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReimportPlan {
    pub changes: Vec<InventoryChange>,
}

impl ReimportPlan {
    /// IDs whose source data should be imported; missing objects are report-only.
    pub fn import_ids(&self) -> Vec<String> {
        self.changes
            .iter()
            .filter(|change| matches!(change.kind, ChangeKind::New | ChangeKind::Changed))
            .map(|change| {
                if change.object_kind == "data_source" {
                    change
                        .parent_id
                        .clone()
                        .unwrap_or_else(|| change.object_id.clone())
                } else {
                    change.object_id.clone()
                }
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// A plan with no creates or updates. Missing-source objects remain visible
    /// in `changes` and do not make the import destructive.
    pub fn is_noop(&self) -> bool {
        self.import_ids().is_empty()
    }
}

/// A data-source move recorded in the manifest while preserving the source ID.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MovedDataSource {
    pub data_source_id: String,
    pub from_database_id: String,
    pub to_database_id: String,
}

/// Compare two complete API inventories. IDs are stable identity; source
/// timestamps and sanitized source content determine whether an existing
/// object needs import. Results are sorted by object ID for reproducibility.
pub fn plan_reimport(previous: &Inventory, current: &Inventory) -> Result<ReimportPlan> {
    let old = inventory_index(previous)?;
    let new = inventory_index(current)?;
    let ids: BTreeSet<String> = old.keys().chain(new.keys()).cloned().collect();
    let mut changes = Vec::with_capacity(ids.len());
    for id in ids {
        let (object_kind, parent_id, kind) = match (old.get(&id), new.get(&id)) {
            (None, Some(object)) => (
                object.kind.clone(),
                object.parent_id.clone(),
                ChangeKind::New,
            ),
            (Some(object), None) => (
                object.kind.clone(),
                object.parent_id.clone(),
                ChangeKind::MissingFromSource,
            ),
            (Some(before), Some(after)) => (
                after.kind.clone(),
                after.parent_id.clone(),
                if object_changed(before, after)? {
                    ChangeKind::Changed
                } else {
                    ChangeKind::Unchanged
                },
            ),
            (None, None) => continue,
        };
        changes.push(InventoryChange {
            object_id: id,
            object_kind,
            parent_id,
            kind,
        });
    }
    Ok(ReimportPlan { changes })
}

/// Carry stable identities and prior conversion records into a fresh
/// inventory, then re-home claims and row membership for data sources whose
/// parent database changed. Source deletions remain in the old identity and
/// row ledgers; this function never deletes or tombstones an ID object.
pub fn rehome_moved_sources(
    previous: &Manifest,
    current: &mut Manifest,
) -> Result<Vec<MovedDataSource>> {
    if previous.workspace_id != current.workspace_id {
        bail!(
            "cannot re-import workspace {} from manifest for {}",
            current.workspace_id,
            previous.workspace_id
        );
    }

    // Work on a copy so malformed inventories or missing destination identities
    // cannot leave the caller with a partially reconciled manifest.
    let mut next = current.clone();
    let old_parents = source_parents(&previous.inventory)?;
    let new_parents = source_parents(&next.inventory)?;
    let mut moves = Vec::new();
    for (source_id, new_database_id) in &new_parents {
        if let Some(old_database_id) = old_parents.get(source_id)
            && old_database_id != new_database_id
        {
            moves.push(MovedDataSource {
                data_source_id: source_id.clone(),
                from_database_id: old_database_id.clone(),
                to_database_id: new_database_id.clone(),
            });
        }
    }

    // Stable source IDs keep their identity; only the parent database is
    // mutable membership metadata. Preserve identities for missing sources too.
    for (source_id, (old_database_id, old_identity)) in &previous.data_source_id_to_identity {
        let database_id = new_parents
            .get(source_id)
            .cloned()
            .unwrap_or_else(|| old_database_id.clone());
        next.data_source_id_to_identity
            .insert(source_id.clone(), (database_id, old_identity.clone()));
    }

    // Existing pages/databases keep their destination note IDs, including
    // source objects now missing from the re-crawled inventory.
    for (id, note_id) in &previous.page_id_to_note_id {
        next.page_id_to_note_id.insert(id.clone(), note_id.clone());
    }
    for (id, note_id) in &previous.database_id_to_note_id {
        next.database_id_to_note_id
            .insert(id.clone(), note_id.clone());
    }

    // Retain earlier row records. For rows in a moved source, update only the
    // database membership reference; page identity and imported row values stay
    // untouched until the caller processes the changed row as an import.
    for (page_id, record) in &previous.row_records {
        next.row_records
            .entry(page_id.clone())
            .or_insert_with(|| record.clone());
    }
    for (page_id, row) in &mut next.row_records {
        let Some(source) = next.inventory.objects.get(page_id) else {
            continue;
        };
        if source.kind != "page" {
            continue;
        }
        let Some(source_id) = source
            .source
            .get("parent")
            .filter(|parent| parent.get("type").and_then(Value::as_str) == Some("data_source_id"))
            .and_then(|parent| parent.get("data_source_id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(database_id) = new_parents.get(source_id) else {
            continue;
        };
        if old_parents
            .get(source_id)
            .is_some_and(|old| old != database_id)
        {
            let database_note_id = next
                .database_id_to_note_id
                .get(database_id)
                .with_context(|| format!("moved data source {source_id} has no note identity for database {database_id}"))?
                .clone();
            row.database_id = database_id.clone();
            row.database_note_id = database_note_id;
        }
    }

    // Merge persisted claim history. Legacy manifests did not store it, so
    // reconstruct their current claims from the old recorded inventory first.
    for record in &previous.schema_claim_history {
        if !next.schema_claim_history.iter().any(|existing| {
            existing.database_id == record.database_id
                && existing.database_note_id == record.database_note_id
                && existing.claim == record.claim
        }) {
            next.schema_claim_history.push(record.clone());
        }
    }
    if previous.schema_claim_history.is_empty() {
        for object in previous
            .inventory
            .objects
            .values()
            .filter(|object| object.kind == "database")
        {
            let Some(database_note_id) = previous.database_id_to_note_id.get(&object.id) else {
                continue;
            };
            for claim in crate::databases::schema_claims_for_database(previous, &object.id)? {
                next.schema_claim_history
                    .push(crate::databases::SchemaClaimRecord {
                        database_id: object.id.clone(),
                        database_note_id: database_note_id.clone(),
                        claim,
                        superseded: false,
                    });
            }
        }
    }

    // Re-convert each destination database once. convert_database records its
    // current claims and supersedes the old database's active claims while
    // retaining them in history.
    let destination_ids: BTreeSet<String> = moves
        .iter()
        .map(|movement| movement.to_database_id.clone())
        .collect();
    for database_id in destination_ids {
        convert_database(&mut next, &database_id)?;
    }
    for movement in &moves {
        if !next.moved_data_sources.contains(movement) {
            next.moved_data_sources.push(movement.clone());
        }
    }

    // The old and new assignments must remain globally unique after identities
    // are carried forward. Fail closed rather than silently forking a note.
    let mut note_ids = BTreeSet::new();
    for note_id in next
        .database_id_to_note_id
        .values()
        .chain(next.page_id_to_note_id.values())
    {
        if !note_ids.insert(note_id) {
            bail!("re-import contains duplicate destination note identity {note_id}");
        }
    }

    *current = next;
    Ok(moves)
}

fn inventory_index(inventory: &Inventory) -> Result<BTreeMap<String, &InventoryObject>> {
    let mut objects = BTreeMap::new();
    for object in inventory.objects.values() {
        insert_inventory_object(&mut objects, object)?;
    }
    for (database_id, sources) in &inventory.data_sources {
        for source in sources {
            if source.kind != "data_source" || source.parent_id.as_deref() != Some(database_id) {
                bail!(
                    "data source {} has inconsistent database membership under {database_id}",
                    source.id
                );
            }
            insert_inventory_object(&mut objects, source)?;
        }
    }
    Ok(objects)
}

fn insert_inventory_object<'a>(
    objects: &mut BTreeMap<String, &'a InventoryObject>,
    object: &'a InventoryObject,
) -> Result<()> {
    if let Some(previous) = objects.insert(object.id.clone(), object) {
        bail!(
            "inventory repeats object identity {} ({}) and ({})",
            object.id,
            previous.kind,
            object.kind
        );
    }
    Ok(())
}

fn source_parents(inventory: &Inventory) -> Result<BTreeMap<String, String>> {
    let mut parents = BTreeMap::new();
    for (database_id, sources) in &inventory.data_sources {
        for source in sources {
            if source.kind != "data_source" || source.parent_id.as_deref() != Some(database_id) {
                bail!(
                    "data source {} has inconsistent database membership under {database_id}",
                    source.id
                );
            }
            if let Some(previous) = parents.insert(source.id.clone(), database_id.clone()) {
                bail!(
                    "data source {} is inventoried under both {previous} and {database_id}",
                    source.id
                );
            }
        }
    }
    Ok(parents)
}

fn object_changed(previous: &InventoryObject, current: &InventoryObject) -> Result<bool> {
    if previous.kind != current.kind || previous.parent_id != current.parent_id {
        return Ok(true);
    }
    let previous_time = previous
        .source
        .get("last_edited_time")
        .and_then(Value::as_str);
    let current_time = current
        .source
        .get("last_edited_time")
        .and_then(Value::as_str);
    let timestamp_advanced =
        matches!((previous_time, current_time), (Some(old), Some(new)) if new > old);
    Ok(timestamp_advanced || content_hash(&previous.source)? != content_hash(&current.source)?)
}

fn content_hash(source: &Value) -> Result<[u8; 32]> {
    let mut content = source.clone();
    if let Value::Object(fields) = &mut content {
        // The timestamp is an independent change signal. Excluding it here
        // means a timestamp regression alone is not misclassified as content.
        fields.remove("last_edited_time");
    }
    let encoded =
        serde_json::to_vec(&content).context("serialize source object for re-import hash")?;
    Ok(*blake3::hash(&encoded).as_bytes())
}
