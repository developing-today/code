use std::collections::BTreeMap;

use id_import_notion::{Inventory, Manifest, NOTION_API_VERSION};
use tempfile::tempdir;

#[test]
fn recorded_responses_yield_expected_inventory_subset() {
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

    assert_eq!(
        inventory.objects.keys().cloned().collect::<Vec<_>>(),
        ["db-alpha", "page-root"]
    );
    assert_eq!(inventory.visible_shares, ["db-alpha", "page-root"]);
    assert_eq!(
        inventory.data_sources["db-alpha"]
            .iter()
            .map(|source| source.id.as_str())
            .collect::<Vec<_>>(),
        ["source-alpha", "source-beta"]
    );
    assert_eq!(
        inventory.data_sources["db-alpha"][0].source["properties"]["Name"]["type"],
        "title"
    );
    assert_eq!(
        inventory.objects["page-root"].source["cover"]["type"],
        "file"
    );
    assert!(
        inventory.objects["page-root"].source["cover"]["file"]
            .get("url")
            .is_none()
    );
    assert!(inventory.comments_available["page-root"]);

    let manifest = Manifest::new(
        "workspace-alpha".into(),
        vec!["api".into()],
        NOTION_API_VERSION.into(),
        inventory,
    )
    .unwrap();
    assert_eq!(manifest.objects.len(), manifest.inventory.objects.len());
    manifest.validate_inventory_reconciles().unwrap();
    assert!(
        manifest
            .data_source_id_to_identity
            .contains_key("source-beta")
    );
    let dir = tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    manifest.save_atomic(&path).unwrap();
    let persisted = std::fs::read_to_string(path).unwrap();
    assert!(!persisted.contains("signed-secret"));
    assert!(!persisted.contains("fixture-secret"));
}
