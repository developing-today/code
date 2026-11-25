//! The tier map `tiers`, third part of slice 9111-03 (#9111 record §3.5,
//! §6.3, §6.4).
//!
//! Oracle: the record's rules, stated before the code. With no `tiers`
//! section every record is in the built-in `default` tier, placed locally
//! and kept indefinitely. A document maps tier identifiers to a placement
//! (`local` or `object_store`) and a retention (`indefinite` or `<n>d`);
//! an absent key or `null` takes the default, unknown keys fail closed.
//! `default` is always in the resolved map. An identifier the map does
//! not name is held in `default` placement and refused for any
//! placement-dependent operation, never reclassified. Object-store
//! placement needs a bound object store, which this build cannot bind
//! yet. A document without `tiers`, or one that only restates the
//! built-in `default`, keeps the digest it had before the map existed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    ConfigDocument, ConfigError, DEFAULT_TIER, FieldClass, Placement, ResolvedConfig, TierLookup,
    explain, field_class, resolved_digest,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-tiers",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        }
    })
}

fn with_tiers(tiers: Value) -> Value {
    let mut doc = base();
    doc["tiers"] = tiers;
    doc
}

fn load(value: &Value) -> Result<ResolvedConfig, ConfigError> {
    ConfigDocument::parse(&value.to_string())?.resolve()
}

fn load_err(value: &Value) -> ConfigError {
    match load(value) {
        Ok(resolved) => panic!("expected rejection, resolved {resolved:?}"),
        Err(err) => err,
    }
}

#[test]
fn an_absent_section_holds_only_the_builtin_default_tier() {
    let resolved = load(&base()).unwrap();
    let tiers = &resolved.tiers;
    assert_eq!(tiers.0.len(), 1);
    let default = tiers.default_tier();
    assert_eq!(default.placement, Placement::Local);
    assert_eq!(default.retention, "indefinite");
    assert_eq!(default.retention_days(), None);
    assert!(tiers.is_default());
}

#[test]
fn a_named_tier_resolves_and_default_is_still_present() {
    let resolved = load(&with_tiers(json!({
        "hot": { "placement": "local", "retention": "30d" },
        "cold": {}
    })))
    .unwrap();
    let tiers = &resolved.tiers;
    assert_eq!(
        tiers.0.keys().map(String::as_str).collect::<Vec<_>>(),
        ["cold", DEFAULT_TIER, "hot"]
    );
    let hot = &tiers.0["hot"];
    assert_eq!(hot.placement, Placement::Local);
    assert_eq!(hot.retention_days(), Some(30));
    let cold = &tiers.0["cold"];
    assert_eq!(cold.placement, Placement::Local);
    assert_eq!(cold.retention, "indefinite");
    assert!(!tiers.is_default());
}

#[test]
fn null_keys_take_the_default_policy() {
    let resolved = load(&with_tiers(json!({
        "warm": { "placement": null, "retention": null }
    })))
    .unwrap();
    let warm = &resolved.tiers.0["warm"];
    assert_eq!(warm.placement, Placement::Local);
    assert_eq!(warm.retention, "indefinite");
}

#[test]
fn a_document_may_restate_or_change_the_default_tier() {
    let restated = load(&with_tiers(json!({
        "default": { "placement": "local", "retention": "indefinite" }
    })))
    .unwrap();
    assert!(restated.tiers.is_default());

    let changed = load(&with_tiers(json!({
        "default": { "retention": "90d" }
    })))
    .unwrap();
    assert_eq!(changed.tiers.0.len(), 1);
    assert_eq!(changed.tiers.default_tier().retention_days(), Some(90));
}

#[test]
fn an_unknown_identifier_is_held_in_default_and_refused_for_placement() {
    let resolved = load(&with_tiers(json!({
        "default": { "retention": "7d" },
        "hot": { "retention": "1d" }
    })))
    .unwrap();
    let tiers = &resolved.tiers;

    let mapped = tiers.lookup("hot");
    assert!(matches!(mapped, TierLookup::Mapped(_)));
    assert!(mapped.allows_placement_dependent_operation());
    assert_eq!(mapped.policy().retention, "1d");

    let unknown = tiers.lookup("archive");
    assert!(matches!(unknown, TierLookup::HeldInDefault(_)));
    assert!(!unknown.allows_placement_dependent_operation());
    // Held under the default tier's policy, not reclassified into another.
    assert_eq!(unknown.policy(), tiers.default_tier());
    assert_eq!(unknown.policy().retention, "7d");
}

#[test]
fn object_store_placement_needs_a_bound_object_store() {
    let err = load_err(&with_tiers(json!({
        "archive": { "placement": "object_store", "retention": "365d" }
    })));
    assert!(
        matches!(&err, ConfigError::TierNeedsObjectStore(tier) if tier == "archive"),
        "{err:?}"
    );
}

#[test]
fn a_bad_retention_rejects_the_document() {
    for retention in [
        "0d",
        "",
        "d",
        "30",
        "30days",
        "-1d",
        "1.5d",
        "forever",
        "99999999999d",
    ] {
        let err = load_err(&with_tiers(json!({ "hot": { "retention": retention } })));
        assert!(
            matches!(
                &err,
                ConfigError::InvalidTierRetention { tier, retention: given }
                    if tier == "hot" && given == retention
            ),
            "{retention:?}: {err:?}"
        );
    }
}

#[test]
fn a_bad_tier_id_rejects_the_document() {
    let long = "a".repeat(65);
    for id in ["", "has space", "dotted.id", "slash/id", "é", long.as_str()] {
        let mut tiers = serde_json::Map::new();
        tiers.insert(id.to_owned(), json!({}));
        let err = load_err(&with_tiers(Value::Object(tiers)));
        assert!(
            matches!(&err, ConfigError::InvalidTierId(given) if given == id),
            "{id:?}: {err:?}"
        );
    }
    let ok = "a".repeat(64);
    let mut tiers = serde_json::Map::new();
    tiers.insert(ok.clone(), json!({}));
    assert!(load(&with_tiers(Value::Object(tiers))).is_ok());
}

#[test]
fn unknown_keys_in_a_tier_fail_closed() {
    let doc = with_tiers(json!({ "hot": { "placement": "local", "grants": ["read"] } }));
    assert!(ConfigDocument::parse(&doc.to_string()).is_err());
    let doc = with_tiers(json!({ "hot": { "placement": "remote" } }));
    assert!(ConfigDocument::parse(&doc.to_string()).is_err());
}

#[test]
fn the_digest_is_unchanged_without_tiers_or_with_only_the_default() {
    let plain = resolved_digest(&load(&base()).unwrap()).unwrap();
    let empty = resolved_digest(&load(&with_tiers(json!({}))).unwrap()).unwrap();
    let restated = resolved_digest(
        &load(&with_tiers(json!({
            "default": { "placement": "local", "retention": "indefinite" }
        })))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plain, empty);
    assert_eq!(plain, restated);

    let resolved = load(&base()).unwrap();
    let canonical: Value = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert!(canonical.get("tiers").is_none());

    let named = resolved_digest(&load(&with_tiers(json!({ "hot": {} }))).unwrap()).unwrap();
    assert_ne!(plain, named);
}

#[test]
fn a_resolved_config_with_tiers_round_trips() {
    let resolved = load(&with_tiers(json!({ "hot": { "retention": "30d" } }))).unwrap();
    let canonical = resolved.canonical_json().unwrap();
    let back: ResolvedConfig = serde_json::from_str(&canonical).unwrap();
    assert_eq!(back, resolved);
}

#[test]
fn explain_lists_each_tier_with_its_source() {
    let doc = with_tiers(json!({ "hot": { "retention": "30d" } })).to_string();
    let explanation = explain(&doc).unwrap();
    let entry = |path: &str| {
        explanation
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no entry {path}"))
    };
    assert_eq!(entry("tiers.hot.retention").value, "30d");
    assert_eq!(entry("tiers.hot.retention").source, "document");
    assert_eq!(entry("tiers.hot.placement").value, "local");
    assert_eq!(entry("tiers.hot.placement").source, "default");
    assert_eq!(entry("tiers.default.placement").source, "default");
    assert_eq!(entry("tiers.default.retention").value, "indefinite");
}

#[test]
fn tiers_is_behavior_and_the_schema_names_it() {
    assert_eq!(field_class("tiers"), Some(FieldClass::Behavior));
    assert_eq!(
        field_class("tiers.hot.placement"),
        Some(FieldClass::Behavior)
    );

    let schema: Value =
        serde_json::from_str(include_str!("../config/id-config-v1.schema.json")).unwrap();
    let tiers = &schema["properties"]["tiers"];
    assert_eq!(tiers["additionalProperties"]["$ref"], json!("#/$defs/tier"));
    assert_eq!(tiers["propertyNames"]["maxLength"], json!(64));
    let tier = &schema["$defs"]["tier"];
    assert_eq!(tier["additionalProperties"], json!(false));
    assert_eq!(
        tier["properties"]["placement"]["enum"],
        json!(["local", "object_store", null])
    );
}
