use std::collections::BTreeMap;

use id_import_notion::{Inventory, Manifest};

#[test]
fn recorded_responses_yield_expected_inventory_subset() {
    let search: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/search.json")).unwrap();
    let data_sources: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/data_sources.json")).unwrap();
    let inventory = Inventory::from_recorded_responses(
        &[search],
        &BTreeMap::from([("db-alpha".into(), vec![data_sources])]),
        vec!["db-alpha".into(), "page-root".into()],
        BTreeMap::from([("page-root".into(), true)]),
        Some("business".into()),
    )
    .unwrap();

    assert_eq!(inventory.objects.len(), 2);
    assert_eq!(inventory.visible_shares, ["db-alpha", "page-root"]);
    assert_eq!(inventory.data_sources["db-alpha"][0].id, "source-alpha");
    assert!(inventory.comments_available["page-root"]);

    let manifest = Manifest::new("workspace-alpha".into(), vec!["api".into()], inventory).unwrap();
    assert_eq!(manifest.objects.len(), manifest.inventory.objects.len());
}
