use std::collections::BTreeSet;

use id_import_notion::{
    Inventory, InventoryObject, Manifest, NOTION_API_VERSION,
    databases::convert_database,
    reimport::{ChangeKind, plan_reimport, rehome_moved_sources},
    rows::convert_row,
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/reimport.json")).unwrap()
}

fn inventory_for(state: &str) -> Inventory {
    let state = fixture()[state].clone();
    let mut inventory = Inventory {
        workspace_edition: Some("business".into()),
        ..Inventory::default()
    };
    for source in state["objects"].as_array().unwrap() {
        let id = source["id"].as_str().unwrap().to_owned();
        let kind = source["object"].as_str().unwrap().to_owned();
        let parent_id = match source["parent"]["type"].as_str() {
            Some("database_id") => source["parent"]["database_id"].as_str(),
            Some("data_source_id") => source["parent"]["data_source_id"].as_str(),
            _ => None,
        }
        .map(str::to_owned);
        inventory.objects.insert(
            id.clone(),
            InventoryObject {
                id,
                kind,
                parent_id,
                source: source.clone(),
            },
        );
    }
    for source in state["data_sources"].as_array().unwrap() {
        let id = source["id"].as_str().unwrap().to_owned();
        let database_id = source["parent"]["database_id"].as_str().unwrap().to_owned();
        inventory
            .data_sources
            .entry(database_id.clone())
            .or_default()
            .push(InventoryObject {
                id,
                kind: "data_source".into(),
                parent_id: Some(database_id),
                source: source.clone(),
            });
    }
    inventory
}

fn manifest_for(state: &str) -> Manifest {
    Manifest::new(
        "workspace-1".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory_for(state),
    )
    .unwrap()
}

fn assign_all_identities(manifest: &mut Manifest, prefix: &str) {
    manifest
        .assign_database_identities(&mut |id| Ok(format!("{prefix}-database-{id}")))
        .unwrap();
    manifest
        .assign_page_identities(&mut |id| Ok(format!("{prefix}-page-{id}")))
        .unwrap();
}

fn prepare_previous() -> Manifest {
    let mut manifest = manifest_for("previous");
    assign_all_identities(&mut manifest, "old");
    convert_database(&mut manifest, "db-old").unwrap();
    let claim_count = manifest.schema_claim_history.len();
    convert_database(&mut manifest, "db-old").unwrap();
    assert_eq!(manifest.schema_claim_history.len(), claim_count);
    for id in ["row-time", "row-hash", "row-missing"] {
        convert_row(&mut manifest, id).unwrap();
    }
    manifest
}

/// Oracle: the frozen previous/current inventories in `fixtures/reimport.json`
/// specify the four exact ID buckets; unchanged IDs must yield no import actions.
#[test]
fn recorded_inventory_diff_is_deterministic_and_unchanged_is_a_noop() {
    let previous = inventory_for("previous");
    let current = inventory_for("current");
    let plan = plan_reimport(&previous, &current).unwrap();
    let by_kind = |kind| -> BTreeSet<String> {
        plan.changes
            .iter()
            .filter(|change| change.kind == kind)
            .map(|change| change.object_id.clone())
            .collect()
    };

    assert_eq!(
        by_kind(ChangeKind::New),
        BTreeSet::from(["db-new".into(), "row-new".into()])
    );
    assert_eq!(
        by_kind(ChangeKind::Changed),
        BTreeSet::from([
            "db-old".into(),
            "ds-move".into(),
            "row-hash".into(),
            "row-time".into()
        ])
    );
    assert_eq!(
        by_kind(ChangeKind::Unchanged),
        BTreeSet::from(["ds-stay".into(), "page-unchanged".into()])
    );
    assert_eq!(
        by_kind(ChangeKind::MissingFromSource),
        BTreeSet::from(["row-missing".into()])
    );
    assert_eq!(
        plan.import_ids(),
        vec!["db-new", "db-old", "row-hash", "row-new", "row-time"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    );

    let unchanged = plan_reimport(&previous, &previous).unwrap();
    assert!(unchanged.is_noop());
    assert!(
        unchanged.import_ids().is_empty(),
        "unchanged objects cause zero creates or updates"
    );
}

/// Oracle: the same fixture preserves `ds-move`'s UUID and row page UUIDs
/// while changing only parent database membership and claim supersession.
#[test]
fn recorded_moved_data_source_rehomes_schema_and_row_membership_without_replacing_identity() {
    let previous = prepare_previous();
    let old_row = previous.row_records["row-time"].clone();
    let old_note_id = previous.page_id_to_note_id["row-time"].clone();
    let old_claims = previous.schema_claim_history.clone();
    assert!(
        !old_claims.is_empty(),
        "the prior schema claims are recorded in the manifest"
    );

    let mut current = manifest_for("current");
    current
        .assign_database_identities(&mut |id| Ok(format!("new-database-{id}")))
        .unwrap();
    let moves = rehome_moved_sources(&previous, &mut current).unwrap();
    current.validate_inventory_reconciles().unwrap();

    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].data_source_id, "ds-move");
    assert_eq!(moves[0].from_database_id, "db-old");
    assert_eq!(moves[0].to_database_id, "db-new");
    assert_eq!(current.moved_data_sources, moves);
    assert_eq!(current.data_source_id_to_identity["ds-move"].0, "db-new");
    assert_eq!(current.page_id_to_note_id["row-time"], old_note_id);
    assert_eq!(
        current.database_id_to_note_id["db-old"],
        previous.database_id_to_note_id["db-old"]
    );
    assert_eq!(
        current.database_id_to_note_id["db-new"],
        "new-database-db-new"
    );

    let rehomed_row = &current.row_records["row-time"];
    assert_eq!(rehomed_row.note_id, old_row.note_id);
    assert_eq!(rehomed_row.database_id, "db-new");
    assert_eq!(rehomed_row.database_note_id, "new-database-db-new");
    assert_eq!(
        rehomed_row.claims, old_row.claims,
        "re-homing changes membership only, not row content"
    );
    assert!(
        current.row_records.contains_key("row-missing"),
        "source deletion remains report-only"
    );
    assert!(
        current.page_id_to_note_id.contains_key("row-missing"),
        "a missing source object keeps its stable note identity"
    );

    let old_active = current
        .schema_claim_history
        .iter()
        .filter(|record| record.claim.data_source_id == "ds-move" && !record.superseded)
        .collect::<Vec<_>>();
    let moved_claim = old_active
        .iter()
        .find(|record| record.database_id == "db-new")
        .expect("current schema is claimed on the new database note");
    assert_eq!(moved_claim.database_note_id, "new-database-db-new");
    assert!(
        current.schema_claim_history.iter().any(|record| {
            record.claim.data_source_id == "ds-move"
                && record.database_id == "db-old"
                && record.superseded
        }),
        "old claims remain in history but are superseded"
    );
    assert!(
        current.schema_claim_history.iter().any(|record| {
            record.claim.data_source_id == "ds-stay"
                && record.database_id == "db-old"
                && !record.superseded
        }),
        "unmoved source claims remain current on the old database"
    );
}

/// Oracle: removing the new fields from a serialized prior manifest simulates
/// pre-slice manifests; re-homing must reconstruct old schema claims from its
/// recorded inventory before superseding them.
#[test]
fn legacy_manifest_reconstructs_schema_history_when_a_source_moves() {
    let previous = prepare_previous();
    let mut encoded = serde_json::to_value(&previous).unwrap();
    let fields = encoded.as_object_mut().unwrap();
    fields.remove("schema_claim_history");
    fields.remove("moved_data_sources");
    let previous: Manifest = serde_json::from_value(encoded).unwrap();
    assert!(previous.schema_claim_history.is_empty());

    let mut current = manifest_for("current");
    current
        .assign_database_identities(&mut |id| Ok(format!("new-database-{id}")))
        .unwrap();
    rehome_moved_sources(&previous, &mut current).unwrap();

    assert!(
        current.schema_claim_history.iter().any(|record| {
            record.claim.data_source_id == "ds-move"
                && record.database_id == "db-old"
                && record.superseded
        }),
        "legacy claims are reconstructed, then retained as superseded history"
    );
    assert!(
        current.schema_claim_history.iter().any(|record| {
            record.claim.data_source_id == "ds-move"
                && record.database_id == "db-new"
                && !record.superseded
        }),
        "the moved source's schema is current on its new database note"
    );
}
