//! Database notes and per-data-source schema claims, slice 3 (#13523).
//!
//! Converts inventoried databases to the §2.2 form: one note per database,
//! with each of its data sources' schemas recorded as provenance claims
//! on the database note, scoped by `data_source_id` so two sources' claims
//! are never merged — even where property names (or property ids) collide.
//!
//! This module covers database-note identity and schema claims. Row
//! notes, relations as typed references, rollups/formulas, and system
//! properties live in [`crate::rows`], building on the identities
//! assigned here; `child_database` references resolve from the same
//! identity map in [`crate::content`].

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Manifest, Outcome, Stage};

/// One per-data-source schema provenance claim on a database note (§2.2).
///
/// The claim identity is (`data_source_id`, `property_id`): the same
/// property name or id in another source of the same database is a
/// different claim, never a merged one.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SchemaClaim {
    pub data_source_id: String,
    pub property_id: String,
    pub property_name: String,
    pub property_type: String,
    /// The verbatim property schema object from the data source.
    pub definition: Value,
}

/// The result of converting one inventoried database (§2.2): its note
/// identity plus one schema claim per property of each of its data
/// sources, in source-id then property-name order.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DatabaseConversion {
    pub database_id: String,
    pub note_id: String,
    pub schema_claims: Vec<SchemaClaim>,
}

/// Convert one inventoried database to its §2.2 form, advancing its
/// manifest checkpoint to [`Stage::Content`].
///
/// The caller must have run [`Manifest::assign_database_identities`]
/// first (the §3 two-pass rule, as for pages); converting a database
/// with no assigned note ID fails rather than minting identity
/// mid-content. Conversion is safe to re-run: identities already
/// recorded are kept, and progress never rewinds (§6.3).
pub fn convert_database(manifest: &mut Manifest, database_id: &str) -> Result<DatabaseConversion> {
    let object = manifest
        .inventory
        .objects
        .get(database_id)
        .with_context(|| format!("database {database_id} is not in the inventory"))?;
    if object.kind != "database" {
        bail!(
            "inventory object {database_id} is a {}, not a database",
            object.kind
        );
    }
    let note_id = manifest
        .database_id_to_note_id
        .get(database_id)
        .with_context(|| {
            format!(
                "database {database_id} has no assigned note id; run assign_database_identities first"
            )
        })?
        .clone();
    let sources = manifest
        .inventory
        .data_sources
        .get(database_id)
        .with_context(|| format!("database {database_id} has no enumerated data sources"))?;
    if sources.is_empty() {
        bail!("database {database_id} has no data sources");
    }
    let mut schema_claims = Vec::new();
    for source in sources {
        let properties = source
            .source
            .get("properties")
            .and_then(Value::as_object)
            .with_context(|| format!("data source {} has no properties object", source.id))?;
        let mut seen_property_ids = BTreeSet::new();
        let mut entries: Vec<(&String, &Value)> = properties.iter().collect();
        entries.sort_by(|left, right| left.0.cmp(right.0));
        for (name, definition) in entries {
            let property_id = definition
                .get("id")
                .and_then(Value::as_str)
                .with_context(|| {
                    format!(
                        "data source {} property {name} is missing its id",
                        source.id
                    )
                })?
                .to_owned();
            let property_type = definition
                .get("type")
                .and_then(Value::as_str)
                .with_context(|| {
                    format!(
                        "data source {} property {name} is missing its type",
                        source.id
                    )
                })?
                .to_owned();
            if !seen_property_ids.insert(property_id.clone()) {
                bail!(
                    "data source {} repeats property id {property_id}",
                    source.id
                );
            }
            schema_claims.push(SchemaClaim {
                data_source_id: source.id.clone(),
                property_id,
                property_name: name.clone(),
                property_type,
                definition: definition.clone(),
            });
        }
    }
    advance(manifest, database_id, Stage::Content, Outcome::Clean, None)?;
    Ok(DatabaseConversion {
        database_id: database_id.to_owned(),
        note_id,
        schema_claims,
    })
}

/// Record staged progress without rewinding it (§6.3): re-converting a
/// database whose checkpoint already sits at a later stage refreshes
/// the conversion artifacts but leaves the checkpoint alone.
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{Inventory, NOTION_API_VERSION};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn manifest_for(sources: Vec<Value>) -> Manifest {
        let database = json!({
            "object": "database",
            "id": "db-1",
            "parent": {"type": "workspace", "workspace": true},
            "data_sources": sources.iter().map(|source| {
                json!({"id": source["id"].as_str().unwrap()})
            }).collect::<Vec<_>>(),
        });
        let mut by_database = BTreeMap::new();
        by_database.insert("db-1".into(), sources);
        let inventory = Inventory::from_recorded_responses(
            &[json!({"object":"list","results":[database],"has_more":false})],
            &by_database,
            vec!["db-1".into()],
            BTreeMap::new(),
            None,
        )
        .unwrap();
        Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
            NOTION_API_VERSION.into(),
            inventory,
        )
        .unwrap()
    }

    fn source(id: &str, properties: Value) -> Value {
        json!({
            "object": "data_source",
            "id": id,
            "parent": {"type": "database_id", "database_id": "db-1"},
            "properties": properties,
        })
    }

    #[test]
    fn unassigned_database_note_id_fails_closed() {
        let mut manifest = manifest_for(vec![source(
            "ds-1",
            json!({"Name": {"id": "title", "type": "title", "title": {}}}),
        )]);
        let err = convert_database(&mut manifest, "db-1").unwrap_err();
        assert!(err.to_string().contains("no assigned note id"), "{err:#}");
    }

    #[test]
    fn converting_a_page_as_a_database_fails_closed() {
        let inventory = Inventory::from_recorded_responses(
            &[json!({"object":"list","results":[{"object":"page","id":"p-1"}],"has_more":false})],
            &BTreeMap::new(),
            vec!["p-1".into()],
            BTreeMap::from([("p-1".into(), false)]),
            None,
        )
        .unwrap();
        let mut manifest = Manifest::new(
            "workspace-1".into(),
            vec!["api".into()],
            NOTION_API_VERSION.into(),
            inventory,
        )
        .unwrap();
        let err = convert_database(&mut manifest, "p-1").unwrap_err();
        assert!(err.to_string().contains("not a database"), "{err:#}");
    }

    #[test]
    fn property_without_type_fails_closed() {
        let mut manifest = manifest_for(vec![source("ds-1", json!({"Name": {"id": "title"}}))]);
        manifest
            .assign_database_identities(&mut |id| Ok(format!("note-{id}")))
            .unwrap();
        let err = convert_database(&mut manifest, "db-1").unwrap_err();
        assert!(err.to_string().contains("missing its type"), "{err:#}");
    }

    #[test]
    fn source_without_properties_fails_closed() {
        let mut source = source("ds-1", json!({}));
        source.as_object_mut().unwrap().remove("properties");
        let mut manifest = manifest_for(vec![source]);
        manifest
            .assign_database_identities(&mut |id| Ok(format!("note-{id}")))
            .unwrap();
        let err = convert_database(&mut manifest, "db-1").unwrap_err();
        assert!(err.to_string().contains("no properties object"), "{err:#}");
    }

    #[test]
    fn same_property_id_in_two_sources_stays_two_claims() {
        let mut manifest = manifest_for(vec![
            source(
                "ds-a",
                json!({"Status": {"id": "status", "type": "status", "status": {}}}),
            ),
            source(
                "ds-b",
                json!({"Status": {"id": "status", "type": "select", "select": {"options": []}}}),
            ),
        ]);
        manifest
            .assign_database_identities(&mut |id| Ok(format!("note-{id}")))
            .unwrap();
        let conversion = convert_database(&mut manifest, "db-1").unwrap();
        assert_eq!(conversion.schema_claims.len(), 2);
        assert_eq!(
            conversion.schema_claims[0],
            SchemaClaim {
                data_source_id: "ds-a".into(),
                property_id: "status".into(),
                property_name: "Status".into(),
                property_type: "status".into(),
                definition: json!({"id": "status", "type": "status", "status": {}}),
            }
        );
        assert_eq!(conversion.schema_claims[1].property_type, "select");
    }
}
