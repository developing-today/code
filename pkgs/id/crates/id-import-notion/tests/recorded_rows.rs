use std::collections::{BTreeMap, BTreeSet};

use id_import_notion::{
    Inventory, Manifest, NOTION_API_VERSION, Outcome, Stage,
    databases::convert_database,
    rows::{ExternalisedReason, ExternalisedRelation, RelationType, RowClaimKind, convert_row},
};
use tempfile::tempdir;

/// Design §8 check 8, second half, plus the check-2 relation subset,
/// over the multi-source fixture database (db-alpha, sources alpha +
/// beta): every row's claims validate against its own source's schema
/// and no row's against the other's; the relation defined in only one
/// source (alpha's `Linked`, targeting source-beta) produces typed
/// references for rows of that source only, resolving to rows of its
/// target data source; values naming anything else are externalised
/// with their reason.
///
/// The oracle is the recorded fixture: source-alpha carries Status as
/// `status` and source-beta carries the same property name and id as
/// `select`, so an implementation that reads a row under the wrong
/// source's schema — or resolves relations against the wrong source's
/// rows — fails here. Row-a2's relation names one row of the target
/// source (resolves), one row of its own source (externalised,
/// not-target-source-row), and one page outside the import set
/// (externalised, outside-import-set).
#[test]
fn recorded_multi_source_rows_validate_per_source_and_relations_resolve() {
    let search: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/search.json")).unwrap();
    let database: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/database.json")).unwrap();
    let data_sources: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/rows_data_sources.json")).unwrap();
    let rows: serde_json::Value = serde_json::from_str(include_str!("fixtures/rows.json")).unwrap();
    let sources = data_sources["results"].as_array().unwrap().clone();
    let mut row_pages = Vec::new();
    for source_id in ["source-alpha", "source-beta"] {
        row_pages.extend(rows[source_id].as_array().unwrap().iter().cloned());
    }
    let availability = BTreeMap::from([
        ("page-root".into(), true),
        ("row-a1".into(), true),
        ("row-a2".into(), true),
        ("row-b1".into(), true),
        ("row-b2".into(), true),
    ]);
    let inventory = Inventory::from_recorded_responses(
        &[
            search,
            serde_json::json!({"object":"list","results":[database],"has_more":false}),
            serde_json::json!({"object":"list","results":row_pages,"has_more":false}),
        ],
        &BTreeMap::from([("db-alpha".into(), sources)]),
        vec!["db-alpha".into(), "page-root".into()],
        availability,
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
    assert_eq!(
        manifest
            .assign_page_identities(&mut |page_id| Ok(format!("note-{page_id}")))
            .unwrap(),
        5
    );
    assert_eq!(
        manifest
            .assign_database_identities(&mut |database_id| Ok(format!("note-{database_id}")))
            .unwrap(),
        1
    );

    let database_conversion = convert_database(&mut manifest, "db-alpha").unwrap();
    // Schema claims per source, keyed (data_source_id, property_id).
    let schema_keys: BTreeMap<&str, BTreeSet<(&str, &str)>> = {
        let mut map: BTreeMap<&str, BTreeSet<(&str, &str)>> = BTreeMap::new();
        for claim in &database_conversion.schema_claims {
            map.entry(claim.data_source_id.as_str())
                .or_default()
                .insert((claim.property_id.as_str(), claim.property_type.as_str()));
        }
        map
    };
    assert_eq!(schema_keys["source-alpha"].len(), 9);
    assert_eq!(schema_keys["source-beta"].len(), 3);

    let mut conversions = BTreeMap::new();
    for row_id in ["row-a1", "row-a2", "row-b1", "row-b2"] {
        conversions.insert(row_id, convert_row(&mut manifest, row_id).unwrap());
    }

    // Every row's record names the one database note and its own
    // source; every claim validates against its own source's schema.
    for (row_id, conversion) in &conversions {
        assert_eq!(conversion.database_note_id, "note-db-alpha", "{row_id}");
        assert_eq!(conversion.note_id, format!("note-{row_id}"));
        for claim in &conversion.claims {
            assert_eq!(claim.data_source_id, conversion.data_source_id);
            assert!(
                schema_keys[conversion.data_source_id.as_str()]
                    .contains(&(claim.property_id.as_str(), claim.property_type.as_str())),
                "{row_id} claim {claim:?} must validate against {} schema",
                conversion.data_source_id
            );
        }
        assert_eq!(manifest.objects[*row_id].stage, Stage::References);
    }
    // The colliding Status property discriminates the sources: no
    // row's Status claim validates against the other source's schema.
    for (row_id, other) in [
        ("row-a1", "source-beta"),
        ("row-a2", "source-beta"),
        ("row-b1", "source-alpha"),
        ("row-b2", "source-alpha"),
    ] {
        let status = conversions[row_id]
            .claims
            .iter()
            .find(|claim| claim.property_id == "status")
            .unwrap();
        assert!(
            !schema_keys[other].contains(&("status", status.property_type.as_str())),
            "{row_id} Status claim must not validate against {other}"
        );
    }

    // The relation is defined in source-alpha only: alpha rows produce
    // references, beta rows produce none.
    let a1 = &conversions["row-a1"];
    assert_eq!(a1.references.len(), 1);
    assert_eq!(a1.references[0].target_page_id, "row-b1");
    assert_eq!(a1.references[0].target_note_id, "note-row-b1");
    assert_eq!(
        a1.references[0].reference_type,
        RelationType::Single {
            side: id_import_notion::rows::RelationSide {
                data_source_id: "source-alpha".into(),
                property_id: "linked".into(),
            }
        }
    );
    assert_eq!(a1.externalised, [] as [ExternalisedRelation; 0]);
    for beta_row in ["row-b1", "row-b2"] {
        assert!(conversions[beta_row].references.is_empty(), "{beta_row}");
        assert!(conversions[beta_row].externalised.is_empty(), "{beta_row}");
    }

    // Check-2 relation subset: row-a2's three values are one typed
    // reference and two externalised items with the expected reasons.
    let a2 = &conversions["row-a2"];
    assert_eq!(a2.references.len(), 1);
    assert_eq!(a2.references[0].target_page_id, "row-b2");
    assert_eq!(a2.externalised.len(), 2);
    assert_eq!(a2.externalised[0].target_page_id, "row-a1");
    assert_eq!(
        a2.externalised[0].reason,
        ExternalisedReason::NotTargetSourceRow
    );
    assert_eq!(a2.externalised[1].target_page_id, "row-outside");
    assert_eq!(
        a2.externalised[1].reason,
        ExternalisedReason::OutsideImportSet
    );
    assert_eq!(manifest.objects["row-a2"].outcome, Outcome::Degraded);
    assert_eq!(manifest.objects["row-b1"].outcome, Outcome::Clean);

    // Claim forms on row-a1: rollup and formula are derived claims
    // carrying their verbatim definitions; the unique ID is a system
    // claim; the people value is a plain provenance claim; the button
    // is named in the not-imported accounting, never claimed.
    let claims: BTreeMap<&str, _> = a1
        .claims
        .iter()
        .map(|claim| (claim.property_id.as_str(), claim))
        .collect();
    assert_eq!(claims["linked_count"].kind, RowClaimKind::Derived);
    assert_eq!(
        claims["linked_count"].definition.as_ref().unwrap()["rollup"]["function"],
        serde_json::json!("count")
    );
    assert_eq!(claims["summary"].kind, RowClaimKind::Derived);
    assert_eq!(
        claims["summary"].definition.as_ref().unwrap()["formula"]["expression"],
        serde_json::json!("prop(\"Name\") + \" / \" + prop(\"Status\")")
    );
    assert_eq!(claims["id"].kind, RowClaimKind::System);
    assert_eq!(claims["created"].kind, RowClaimKind::System);
    assert_eq!(claims["owner"].kind, RowClaimKind::Value);
    assert_eq!(claims["status"].kind, RowClaimKind::Value);
    assert!(!claims.contains_key("action"));
    assert_eq!(a1.not_imported.len(), 1);
    assert_eq!(a1.not_imported[0].property_id, "action");
    assert_eq!(a1.not_imported[0].property_type, "button");

    // The row records survive a manifest round-trip unchanged.
    assert_eq!(manifest.row_records.len(), 4);
    let dir = tempdir().unwrap();
    let path = dir.path().join("manifest.json");
    manifest.save_atomic(&path).unwrap();
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded, manifest);
    assert_eq!(loaded.row_records["row-a1"].references.len(), 1);
}
