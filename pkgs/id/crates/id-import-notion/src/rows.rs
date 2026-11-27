//! Row notes and property claims, slice 3 remainder (#13523).
//!
//! Converts inventoried database rows — pages whose parent is a data
//! source — to the §2.2 form: one note per row, a typed reference to
//! the database note, and the row's property values recorded as claims
//! interpreted against the schema of the data source the row was read
//! from. Relations become typed references between row notes (§2.3)
//! whose reference type is the relation's schema identity — (data
//! source, property), never the parent database — in the canonical
//! dual-relation form of §2.2. Rollups and formulas become
//! derived-value provenance claims (computed display value plus the
//! verbatim definition, never silently-authoritative values), system
//! properties become provenance claims, and button / verification
//! properties are named in the conversion's not-imported accounting
//! instead of being imported as values.
//!
//! A row value whose type disagrees with its own source's schema, or
//! that names a property the schema does not carry, fails the
//! conversion closed: reading one source's rows under another source's
//! schema is exactly the conflation failure §2.2 exists to prevent.
//! Relation values resolve only against rows of the schema's target
//! data source; a value naming anything else is recorded as an
//! externalised relation with its reason and counted, never dropped.
//!
//! Records are stored in the manifest keyed by the row's page UUID
//! and replaced wholesale on re-conversion, so an unchanged corpus
//! reproduces identical artifacts. A row's identity is its page UUID
//! alone (§6.1): the database and data source named in its record are
//! membership metadata, updated when a later import reads the row
//! under a new database or source. Superseding a moved data source's
//! prior schema claims on its old database note (§6.1) belongs to the
//! re-import driver, which does not exist in this crate yet; it is
//! the named remainder on #13523.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Manifest, Outcome, Stage};

/// Which §2.2 claim form a property value took.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RowClaimKind {
    /// A plain value claim, typed by the schema (rich text, number,
    /// select, people, files, the title property, …).
    Value,
    /// A rollup or formula: the computed display value plus the
    /// verbatim definition it was computed from.
    Derived,
    /// A system property (unique ID, created/last-edited time or by):
    /// a provenance fact about the source, handled like a page
    /// timestamp.
    System,
}

/// One property value of one row, recorded as a claim against the
/// schema of the data source the row was read from. The claim identity
/// is (`data_source_id`, `property_id`), as for schema claims.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PropertyClaim {
    pub kind: RowClaimKind,
    pub data_source_id: String,
    pub property_id: String,
    pub property_name: String,
    pub property_type: String,
    /// The verbatim property-value object from the row page.
    pub value: Value,
    /// The verbatim schema definition, for
    /// [`RowClaimKind::Derived`] claims (rollup / formula) only: the
    /// definition names the inputs the display value was computed
    /// from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<Value>,
}

/// One side of a relation's schema identity: a (data source, property)
/// pair. The parent database is never part of it (§2.2), so a data
/// source moved to another database keeps the same relation identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationSide {
    pub data_source_id: String,
    pub property_id: String,
}

/// The reference type of a relation (§2.2): a one-way relation's type
/// is its single (data source, property) pair; a two-way (dual)
/// relation's canonical type is the unordered pair of both sides,
/// joined through `dual_property.synced_property_id`, stored sorted
/// so the side that sorts first is the canonical direction and both
/// sides import as the one type.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelationType {
    Single {
        side: RelationSide,
    },
    Dual {
        first: RelationSide,
        second: RelationSide,
    },
}

/// A relation value that resolved to a row of the schema's target
/// data source: a typed reference from the owning row's note (the
/// record this sits in) to the target row's note.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RelationReference {
    pub property_id: String,
    pub property_name: String,
    pub reference_type: RelationType,
    /// Notion UUID of the target row page.
    pub target_page_id: String,
    /// Destination note ID of the target row.
    pub target_note_id: String,
}

/// Why a relation value did not become a typed reference.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExternalisedReason {
    /// The named page is not in the import set at all.
    OutsideImportSet,
    /// The named page is in the import set but is not a row of the
    /// relation schema's target data source, so the value cannot
    /// resolve against the rows the schema names.
    NotTargetSourceRow,
}

/// A relation value that named no row of the target data source,
/// recorded as a provenance claim (the target's UUID) and counted
/// externalised for the fidelity report (§3), never dropped.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExternalisedRelation {
    pub property_id: String,
    pub property_name: String,
    pub reference_type: RelationType,
    pub target_page_id: String,
    pub reason: ExternalisedReason,
}

/// A property the design does not import as a value (button,
/// verification), named in the accounting so the fidelity report can
/// list it per database (§2.2). The schema claim still records that
/// the property existed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NotImportedProperty {
    pub data_source_id: String,
    pub property_id: String,
    pub property_name: String,
    pub property_type: String,
}

/// The result of converting one row page, and the record the manifest
/// keeps for it: the row note's identity, its typed reference to the
/// database note (via `database_note_id`), and every claim, reference,
/// externalised relation, and not-imported property the row produced.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RowConversion {
    pub page_id: String,
    pub note_id: String,
    pub database_id: String,
    pub database_note_id: String,
    pub data_source_id: String,
    pub claims: Vec<PropertyClaim>,
    pub references: Vec<RelationReference>,
    pub externalised: Vec<ExternalisedRelation>,
    pub not_imported: Vec<NotImportedProperty>,
}

/// One schema property of a data source, keyed by property id.
struct SchemaProperty {
    name: String,
    property_type: String,
    definition: Value,
}

/// Index one data source's schema by property id, applying the same
/// fail-closed validation as [`crate::databases::convert_database`].
fn schema_properties(source: &Value, source_id: &str) -> Result<BTreeMap<String, SchemaProperty>> {
    let properties = source
        .get("properties")
        .and_then(Value::as_object)
        .with_context(|| format!("data source {source_id} has no properties object"))?;
    let mut by_id = BTreeMap::new();
    for (name, definition) in properties {
        let property_id = definition
            .get("id")
            .and_then(Value::as_str)
            .with_context(|| format!("data source {source_id} property {name} is missing its id"))?
            .to_owned();
        let property_type = definition
            .get("type")
            .and_then(Value::as_str)
            .with_context(|| {
                format!("data source {source_id} property {name} is missing its type")
            })?
            .to_owned();
        let entry = SchemaProperty {
            name: name.clone(),
            property_type,
            definition: definition.clone(),
        };
        if by_id.insert(property_id.clone(), entry).is_some() {
            bail!("data source {source_id} repeats property id {property_id}");
        }
    }
    Ok(by_id)
}

/// Compute a relation property's canonical reference type (§2.2) and
/// the data source its values resolve against.
fn relation_type(
    schema: &SchemaProperty,
    own_source_id: &str,
    property_id: &str,
) -> Result<(RelationType, String)> {
    let relation = schema
        .definition
        .get("relation")
        .and_then(Value::as_object)
        .with_context(|| {
            format!("relation property {property_id} is missing its relation config")
        })?;
    let target_source_id = relation
        .get("data_source_id")
        .and_then(Value::as_str)
        .with_context(|| {
            format!("relation property {property_id} is missing its target data_source_id")
        })?
        .to_owned();
    let own = RelationSide {
        data_source_id: own_source_id.to_owned(),
        property_id: property_id.to_owned(),
    };
    let reference_type = match relation.get("type").and_then(Value::as_str) {
        None | Some("single_property") => RelationType::Single { side: own },
        Some("dual_property") => {
            let synced_property_id = relation
                .get("dual_property")
                .and_then(|dual| dual.get("synced_property_id"))
                .and_then(Value::as_str)
                .with_context(|| {
                    format!(
                        "dual relation property {property_id} is missing \
                         dual_property.synced_property_id"
                    )
                })?;
            let other = RelationSide {
                data_source_id: target_source_id.clone(),
                property_id: synced_property_id.to_owned(),
            };
            let (first, second) = if own <= other {
                (own, other)
            } else {
                (other, own)
            };
            RelationType::Dual { first, second }
        }
        Some(other) => {
            bail!("relation property {property_id} has unsupported relation type {other}")
        }
    };
    Ok((reference_type, target_source_id))
}

/// Convert one inventoried row page to its §2.2 form, recording the
/// result in the manifest's row records and advancing the row's
/// checkpoint to [`Stage::References`].
///
/// The caller must have run [`Manifest::assign_page_identities`] and
/// [`Manifest::assign_database_identities`] first (the §3 two-pass
/// rule); converting a row with no assigned note ID — or whose
/// database has none — fails rather than minting identity
/// mid-content. Conversion is safe to re-run: the row's record is
/// replaced wholesale and progress never rewinds (§6.3).
pub fn convert_row(manifest: &mut Manifest, page_id: &str) -> Result<RowConversion> {
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
    let source_page = object.source.clone();
    let parent = source_page
        .get("parent")
        .with_context(|| format!("row page {page_id} is missing its parent"))?;
    if parent.get("type").and_then(Value::as_str) != Some("data_source_id") {
        bail!("page {page_id} is not a database row (its parent is not a data source)");
    }
    let data_source_id = parent
        .get("data_source_id")
        .and_then(Value::as_str)
        .with_context(|| format!("row page {page_id} parent is missing data_source_id"))?
        .to_owned();
    let (database_id, schema_source) = manifest
        .inventory
        .data_sources
        .iter()
        .find_map(|(database_id, sources)| {
            sources
                .iter()
                .find(|source| source.id == data_source_id)
                .map(|source| (database_id.clone(), source.source.clone()))
        })
        .with_context(|| {
            format!("data source {data_source_id} is not enumerated in the inventory")
        })?;
    let note_id = manifest
        .page_id_to_note_id
        .get(page_id)
        .with_context(|| {
            format!("page {page_id} has no assigned note id; run assign_page_identities first")
        })?
        .clone();
    let database_note_id = manifest
        .database_id_to_note_id
        .get(&database_id)
        .with_context(|| {
            format!(
                "database {database_id} has no assigned note id; \
                 run assign_database_identities first"
            )
        })?
        .clone();
    let schema = schema_properties(&schema_source, &data_source_id)?;
    let properties = source_page
        .get("properties")
        .and_then(Value::as_object)
        .with_context(|| format!("row page {page_id} has no properties object"))?;
    let mut entries: Vec<(&String, &Value)> = properties.iter().collect();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let mut claims = Vec::new();
    let mut references = Vec::new();
    let mut externalised = Vec::new();
    let mut not_imported = Vec::new();
    for (name, value) in entries {
        let property_id = value
            .get("id")
            .and_then(Value::as_str)
            .with_context(|| format!("row page {page_id} property {name} is missing its id"))?
            .to_owned();
        let value_type = value
            .get("type")
            .and_then(Value::as_str)
            .with_context(|| format!("row page {page_id} property {name} is missing its type"))?
            .to_owned();
        let property = schema.get(&property_id).with_context(|| {
            format!(
                "row page {page_id} property {name} ({property_id}) is not in the \
                 schema of data source {data_source_id}"
            )
        })?;
        if value_type != property.property_type {
            bail!(
                "row page {page_id} property {} has type {value_type} but the schema \
                 of data source {data_source_id} says {}",
                property.name,
                property.property_type
            );
        }
        let claim = |kind, definition| PropertyClaim {
            kind,
            data_source_id: data_source_id.clone(),
            property_id: property_id.clone(),
            property_name: property.name.clone(),
            property_type: property.property_type.clone(),
            value: value.clone(),
            definition,
        };
        match property.property_type.as_str() {
            "relation" => {
                let (reference_type, target_source_id) =
                    relation_type(property, &data_source_id, &property_id)?;
                let targets = value
                    .get("relation")
                    .and_then(Value::as_array)
                    .with_context(|| {
                        format!(
                            "row page {page_id} relation property {property_id} \
                             is missing its relation array"
                        )
                    })?;
                for target in targets {
                    let target_page_id = target
                        .get("id")
                        .and_then(Value::as_str)
                        .with_context(|| {
                            format!(
                                "row page {page_id} relation property {property_id} \
                                 names a target without an id"
                            )
                        })?
                        .to_owned();
                    let target_row = manifest.inventory.objects.get(&target_page_id);
                    let resolves = target_row.is_some_and(|target_object| {
                        target_object.kind == "page"
                            && target_object
                                .source
                                .get("parent")
                                .and_then(|target_parent| target_parent.get("data_source_id"))
                                .and_then(Value::as_str)
                                == Some(target_source_id.as_str())
                    });
                    if target_row.is_none() {
                        externalised.push(ExternalisedRelation {
                            property_id: property_id.clone(),
                            property_name: property.name.clone(),
                            reference_type: reference_type.clone(),
                            target_page_id,
                            reason: ExternalisedReason::OutsideImportSet,
                        });
                    } else if resolves {
                        let target_note_id = manifest
                            .page_id_to_note_id
                            .get(&target_page_id)
                            .with_context(|| {
                                format!(
                                    "relation target row {target_page_id} has no \
                                     assigned note id; run assign_page_identities first"
                                )
                            })?
                            .clone();
                        references.push(RelationReference {
                            property_id: property_id.clone(),
                            property_name: property.name.clone(),
                            reference_type: reference_type.clone(),
                            target_page_id,
                            target_note_id,
                        });
                    } else {
                        externalised.push(ExternalisedRelation {
                            property_id: property_id.clone(),
                            property_name: property.name.clone(),
                            reference_type: reference_type.clone(),
                            target_page_id,
                            reason: ExternalisedReason::NotTargetSourceRow,
                        });
                    }
                }
            }
            "rollup" | "formula" => {
                claims.push(claim(
                    RowClaimKind::Derived,
                    Some(property.definition.clone()),
                ));
            }
            "unique_id" | "created_time" | "last_edited_time" | "created_by" | "last_edited_by" => {
                claims.push(claim(RowClaimKind::System, None));
            }
            "button" | "verification" => {
                not_imported.push(NotImportedProperty {
                    data_source_id: data_source_id.clone(),
                    property_id: property_id.clone(),
                    property_name: property.name.clone(),
                    property_type: property.property_type.clone(),
                });
            }
            _ => {
                claims.push(claim(RowClaimKind::Value, None));
            }
        }
    }
    let conversion = RowConversion {
        page_id: page_id.to_owned(),
        note_id,
        database_id,
        database_note_id,
        data_source_id,
        claims,
        references,
        externalised,
        not_imported,
    };
    manifest
        .row_records
        .insert(page_id.to_owned(), conversion.clone());
    let outcome = if conversion.externalised.is_empty() && conversion.not_imported.is_empty() {
        Outcome::Clean
    } else {
        Outcome::Degraded
    };
    advance(manifest, page_id, Stage::References, outcome, None)?;
    Ok(conversion)
}

/// Record staged progress without rewinding it (§6.3), as in
/// [`crate::databases`].
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

    fn source(id: &str, properties: Value) -> Value {
        json!({
            "object": "data_source",
            "id": id,
            "parent": {"type": "database_id", "database_id": "db-1"},
            "properties": properties,
        })
    }

    fn row(id: &str, source_id: &str, properties: Value) -> Value {
        json!({
            "object": "page",
            "id": id,
            "parent": {"type": "data_source_id", "data_source_id": source_id},
            "properties": properties,
        })
    }

    fn manifest_for(sources: Vec<Value>, pages: Vec<Value>) -> Manifest {
        let database = json!({
            "object": "database",
            "id": "db-1",
            "parent": {"type": "workspace", "workspace": true},
            "data_sources": sources.iter().map(|source| {
                json!({"id": source["id"].as_str().unwrap()})
            }).collect::<Vec<_>>(),
        });
        let mut results = vec![database];
        results.extend(pages);
        let mut by_database = BTreeMap::new();
        by_database.insert("db-1".into(), sources);
        let inventory = Inventory::from_recorded_responses(
            &[json!({"object":"list","results":results,"has_more":false})],
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

    fn assigned(manifest: &mut Manifest) {
        manifest
            .assign_page_identities(&mut |id| Ok(format!("note-{id}")))
            .unwrap();
        manifest
            .assign_database_identities(&mut |id| Ok(format!("note-{id}")))
            .unwrap();
    }

    fn title_property() -> Value {
        json!({"Name": {"id": "title", "type": "title", "title": {}}})
    }

    fn title_value(text: &str) -> Value {
        json!([{"type": "text", "plain_text": text}])
    }

    #[test]
    fn plain_page_is_not_a_row() {
        let page = json!({
            "object": "page",
            "id": "p-1",
            "parent": {"type": "workspace", "workspace": true},
            "properties": {},
        });
        let mut manifest = manifest_for(vec![source("ds-1", title_property())], vec![page]);
        assigned(&mut manifest);
        let err = convert_row(&mut manifest, "p-1").unwrap_err();
        assert!(err.to_string().contains("not a database row"), "{err:#}");
    }

    #[test]
    fn unassigned_row_note_id_fails_closed() {
        let mut manifest = manifest_for(
            vec![source("ds-1", title_property())],
            vec![row(
                "row-1",
                "ds-1",
                json!({"Name": {"id": "title", "type": "title", "title": title_value("One")}}),
            )],
        );
        let err = convert_row(&mut manifest, "row-1").unwrap_err();
        assert!(err.to_string().contains("no assigned note id"), "{err:#}");
    }

    #[test]
    fn value_type_mismatch_against_own_source_fails_closed() {
        // The schema says Status is a select; the row carries it with
        // the *other* source's type (status). Interpreting the row
        // under its own source's schema must refuse, not coerce.
        let mut manifest = manifest_for(
            vec![source(
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": {}},
                    "Status": {"id": "status", "type": "select", "select": {"options": []}},
                }),
            )],
            vec![row(
                "row-1",
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": title_value("One")},
                    "Status": {"id": "status", "type": "status", "status": {"name": "Doing"}},
                }),
            )],
        );
        assigned(&mut manifest);
        let err = convert_row(&mut manifest, "row-1").unwrap_err();
        assert!(err.to_string().contains("says select"), "{err:#}");
    }

    #[test]
    fn property_absent_from_schema_fails_closed() {
        let mut manifest = manifest_for(
            vec![source("ds-1", title_property())],
            vec![row(
                "row-1",
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": title_value("One")},
                    "Ghost": {"id": "ghost", "type": "number", "number": 1},
                }),
            )],
        );
        assigned(&mut manifest);
        let err = convert_row(&mut manifest, "row-1").unwrap_err();
        assert!(err.to_string().contains("is not in the schema"), "{err:#}");
    }

    #[test]
    fn dual_relation_canonical_type_is_shared_by_both_sides() {
        let alpha = source(
            "ds-a",
            json!({
                "Name": {"id": "title", "type": "title", "title": {}},
                "Rel": {"id": "rel", "type": "relation", "relation": {
                    "data_source_id": "ds-b", "type": "dual_property",
                    "dual_property": {"synced_property_id": "rel_back", "synced_property_name": "RelBack"}
                }},
            }),
        );
        let beta = source(
            "ds-b",
            json!({
                "Name": {"id": "title", "type": "title", "title": {}},
                "RelBack": {"id": "rel_back", "type": "relation", "relation": {
                    "data_source_id": "ds-a", "type": "dual_property",
                    "dual_property": {"synced_property_id": "rel", "synced_property_name": "Rel"}
                }},
            }),
        );
        // Both sources belong to the one database in this fixture; the
        // relation identity is keyed by data source, not database.
        let mut manifest = manifest_for(
            vec![alpha, beta],
            vec![
                row(
                    "row-a",
                    "ds-a",
                    json!({
                        "Name": {"id": "title", "type": "title", "title": title_value("A")},
                        "Rel": {"id": "rel", "type": "relation", "relation": [{"id": "row-b"}]},
                    }),
                ),
                row(
                    "row-b",
                    "ds-b",
                    json!({
                        "Name": {"id": "title", "type": "title", "title": title_value("B")},
                        "RelBack": {"id": "rel_back", "type": "relation", "relation": [{"id": "row-a"}]},
                    }),
                ),
            ],
        );
        assigned(&mut manifest);
        let expected = RelationType::Dual {
            first: RelationSide {
                data_source_id: "ds-a".into(),
                property_id: "rel".into(),
            },
            second: RelationSide {
                data_source_id: "ds-b".into(),
                property_id: "rel_back".into(),
            },
        };
        let from_a = convert_row(&mut manifest, "row-a").unwrap();
        assert_eq!(from_a.references.len(), 1);
        assert_eq!(from_a.references[0].reference_type, expected);
        assert_eq!(from_a.references[0].target_note_id, "note-row-b");
        let from_b = convert_row(&mut manifest, "row-b").unwrap();
        assert_eq!(from_b.references.len(), 1);
        // Both sides import as the one canonical type: no duplication.
        assert_eq!(from_b.references[0].reference_type, expected);
        assert_eq!(from_b.references[0].target_note_id, "note-row-a");
    }

    #[test]
    fn relation_value_outside_import_set_is_externalised() {
        let mut manifest = manifest_for(
            vec![source(
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": {}},
                    "Rel": {"id": "rel", "type": "relation", "relation": {
                        "data_source_id": "ds-1", "type": "single_property", "single_property": {}
                    }},
                }),
            )],
            vec![row(
                "row-1",
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": title_value("One")},
                    "Rel": {"id": "rel", "type": "relation", "relation": [{"id": "row-ghost"}]},
                }),
            )],
        );
        assigned(&mut manifest);
        let conversion = convert_row(&mut manifest, "row-1").unwrap();
        assert_eq!(conversion.references, [] as [RelationReference; 0]);
        assert_eq!(conversion.externalised.len(), 1);
        assert_eq!(
            conversion.externalised[0].reason,
            ExternalisedReason::OutsideImportSet
        );
        assert_eq!(conversion.externalised[0].target_page_id, "row-ghost");
        assert_eq!(
            conversion.externalised[0].reference_type,
            RelationType::Single {
                side: RelationSide {
                    data_source_id: "ds-1".into(),
                    property_id: "rel".into(),
                }
            }
        );
        assert_eq!(manifest.objects["row-1"].outcome, Outcome::Degraded);
    }

    #[test]
    fn derived_system_and_not_imported_property_forms() {
        let mut manifest = manifest_for(
            vec![source(
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": {}},
                    "Count": {"id": "count", "type": "rollup", "rollup": {
                        "relation_property_id": "rel", "rollup_property_id": "title", "function": "count"
                    }},
                    "Label": {"id": "label", "type": "formula", "formula": {"expression": "prop(\"Name\")"}},
                    "ID": {"id": "id", "type": "unique_id", "unique_id": {"prefix": "T"}},
                    "Action": {"id": "action", "type": "button", "button": {}},
                }),
            )],
            vec![row(
                "row-1",
                "ds-1",
                json!({
                    "Name": {"id": "title", "type": "title", "title": title_value("One")},
                    "Count": {"id": "count", "type": "rollup", "rollup": {"type": "number", "number": 2}},
                    "Label": {"id": "label", "type": "formula", "formula": {"type": "string", "string": "One"}},
                    "ID": {"id": "id", "type": "unique_id", "unique_id": {"prefix": "T", "number": 3}},
                    "Action": {"id": "action", "type": "button", "button": {}},
                }),
            )],
        );
        assigned(&mut manifest);
        let conversion = convert_row(&mut manifest, "row-1").unwrap();
        let by_id: BTreeMap<&str, &PropertyClaim> = conversion
            .claims
            .iter()
            .map(|claim| (claim.property_id.as_str(), claim))
            .collect();
        assert_eq!(by_id["count"].kind, RowClaimKind::Derived);
        assert_eq!(
            by_id["count"].definition.as_ref().unwrap()["rollup"]["function"],
            json!("count")
        );
        assert_eq!(by_id["label"].kind, RowClaimKind::Derived);
        assert_eq!(
            by_id["label"].definition.as_ref().unwrap()["formula"]["expression"],
            json!("prop(\"Name\")")
        );
        assert_eq!(by_id["id"].kind, RowClaimKind::System);
        assert_eq!(by_id["title"].kind, RowClaimKind::Value);
        assert_eq!(conversion.not_imported.len(), 1);
        assert_eq!(conversion.not_imported[0].property_id, "action");
        assert_eq!(conversion.not_imported[0].property_type, "button");
        assert!(!by_id.contains_key("action"));
    }

    #[test]
    fn rerun_reproduces_identical_record() {
        let mut manifest = manifest_for(
            vec![source("ds-1", title_property())],
            vec![row(
                "row-1",
                "ds-1",
                json!({"Name": {"id": "title", "type": "title", "title": title_value("One")}}),
            )],
        );
        assigned(&mut manifest);
        let first = convert_row(&mut manifest, "row-1").unwrap();
        let second = convert_row(&mut manifest, "row-1").unwrap();
        assert_eq!(first, second);
        assert_eq!(manifest.row_records.len(), 1);
        assert_eq!(manifest.row_records["row-1"], first);
        assert_eq!(manifest.objects["row-1"].stage, Stage::References);
    }
}
