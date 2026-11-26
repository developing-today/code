use std::collections::{BTreeMap, BTreeSet};

use id_import_notion::{
    Inventory, Manifest, NOTION_API_VERSION, Stage, databases::convert_database,
};
use tempfile::tempdir;

/// Design §8 check 8, first half: the multi-source fixture database
/// (db-alpha, sources alpha + beta) converts to one database note whose
/// schema claims are disjoint per data source. The oracle is the
/// recorded fixture: source-alpha carries Status as `status`, source-beta
/// carries the same property name as `select`, both under property id
/// `status` — so any implementation that keys claims by property id or
/// name alone merges them and fails this test.
#[test]
fn recorded_multi_source_database_yields_disjoint_schema_claims() {
    let search: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/search.json")).unwrap();
    let data_sources: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/data_sources.json")).unwrap();
    let database: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/database.json")).unwrap();
    let sources = data_sources["results"].as_array().unwrap().clone();
    let inventory = Inventory::from_recorded_responses(
        &[
            search,
            serde_json::json!({"object":"list","results":[database],"has_more":false}),
        ],
        &BTreeMap::from([("db-alpha".into(), sources)]),
        vec!["db-alpha".into(), "page-root".into()],
        BTreeMap::from([("page-root".into(), true)]),
        Some("business".into()),
    )
    .unwrap();
    let mut manifest = Manifest::new(
        "workspace-alpha".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap();

    let assigned = manifest
        .assign_database_identities(&mut |database_id| Ok(format!("note-{database_id}")))
        .unwrap();
    assert_eq!(assigned, 1);
    assert_eq!(manifest.database_id_to_note_id["db-alpha"], "note-db-alpha");

    let conversion = convert_database(&mut manifest, "db-alpha").unwrap();
    assert_eq!(conversion.database_id, "db-alpha");
    assert_eq!(conversion.note_id, "note-db-alpha");

    // (a) The manifest records both data_source_ids under the one
    // database note: both sources enumerate under db-alpha.
    assert_eq!(manifest.inventory.data_sources["db-alpha"].len(), 2);
    let claimed_sources: BTreeSet<&str> = conversion
        .schema_claims
        .iter()
        .map(|claim| claim.data_source_id.as_str())
        .collect();
    assert_eq!(
        claimed_sources,
        BTreeSet::from(["source-alpha", "source-beta"])
    );

    // (b) The colliding property name appears once per source, with each
    // source's type — the claims are disjoint, never merged.
    let status: Vec<_> = conversion
        .schema_claims
        .iter()
        .filter(|claim| claim.property_name == "Status")
        .collect();
    assert_eq!(status.len(), 2);
    assert!(
        status
            .iter()
            .any(|claim| claim.data_source_id == "source-alpha" && claim.property_type == "status"),
        "source-alpha Status claim: {status:?}"
    );
    assert!(
        status
            .iter()
            .any(|claim| claim.data_source_id == "source-beta" && claim.property_type == "select"),
        "source-beta Status claim: {status:?}"
    );
    // Same property id in both sources stays two claims: the claim
    // identity is (data_source_id, property_id).
    let status_keys: BTreeSet<(&str, &str)> = status
        .iter()
        .map(|claim| (claim.data_source_id.as_str(), claim.property_id.as_str()))
        .collect();
    assert_eq!(status_keys.len(), 2);

    // Every claim key is unique and every definition round-trips its type.
    let keys: BTreeSet<(&str, &str)> = conversion
        .schema_claims
        .iter()
        .map(|claim| (claim.data_source_id.as_str(), claim.property_id.as_str()))
        .collect();
    assert_eq!(keys.len(), conversion.schema_claims.len());
    assert_eq!(conversion.schema_claims.len(), 5);
    for claim in &conversion.schema_claims {
        assert_eq!(
            claim.definition["type"].as_str(),
            Some(claim.property_type.as_str()),
            "claim definition must round-trip its type: {claim:?}"
        );
    }

    // Conversion advances the database checkpoint to Content, and the
    // new identity map survives a manifest round-trip.
    assert_eq!(manifest.objects["db-alpha"].stage, Stage::Content);
    let dir = tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    manifest.save_atomic(&path).unwrap();
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded.database_id_to_note_id["db-alpha"], "note-db-alpha");
    assert_eq!(loaded, manifest);
}
