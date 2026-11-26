//! The link plane `links`, fifth part of slice 9111-03 (#9111 record
//! §3.3, §6.3, §6.4, §6.5 step 4).
//!
//! Oracle: the record's rules, stated before the code. Without `links`,
//! every configurable kind (`sync`, `query`, `send_only`) is off, on the
//! `iroh` adapter with the `default` retention class; the sync scope falls
//! to `behavior.sync_scope` and the query timeout to the fan-out per-peer
//! timeout. An absent key and `null` both take the default. Unknown keys
//! fail closed, and the `control` link is always on with no keys: `{}`
//! is accepted, any key in it rejects. An adapter this build does not
//! advertise is a load error. A document without `links`, or one that
//! restates the defaults, keeps its digest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    ADVERTISED_LINK_ADAPTERS, ConfigDocument, ConfigError, FieldClass, ResolvedConfig, SyncScope,
    explain, field_class, resolved_digest,
};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-links",
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

fn with_links(links: Value) -> Value {
    let mut doc = base();
    doc["links"] = links;
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
fn an_absent_section_turns_every_kind_off() {
    let resolved = load(&base()).unwrap();
    let links = &resolved.links;
    assert!(!links.sync.enabled);
    assert!(!links.query.enabled);
    assert!(!links.send_only.enabled);
    assert_eq!(links.sync.adapter, "iroh");
    assert_eq!(links.send_only.adapter, "iroh");
    assert_eq!(links.sync.retention_class, "default");
    assert_eq!(links.send_only.retention_class, "default");
    assert_eq!(links.sync.sync_scope, None);
    assert_eq!(links.query.timeout_ms, None);
    assert!(links.is_default());
    assert_eq!(ADVERTISED_LINK_ADAPTERS, ["iroh"]);
}

#[test]
fn written_values_resolve_and_null_takes_the_default() {
    let resolved = load(&with_links(json!({
        "sync": { "enabled": true, "sync_scope": "metadata", "retention_class": "short" },
        "query": { "enabled": true, "timeout_ms": 750 },
        "send_only": { "enabled": null, "adapter": null, "retention_class": null }
    })))
    .unwrap();
    let links = &resolved.links;
    assert!(links.sync.enabled);
    assert_eq!(links.sync.sync_scope, Some(SyncScope::Metadata));
    assert_eq!(links.sync.retention_class, "short");
    assert!(links.query.enabled);
    assert_eq!(links.query.timeout_ms, Some(750));
    assert!(!links.send_only.enabled);
    assert_eq!(links.send_only.adapter, "iroh");
    assert!(!links.is_default());
}

#[test]
fn an_unadvertised_adapter_rejects_the_document() {
    for link in ["sync", "send_only"] {
        let err = load_err(&with_links(
            json!({ link: { "adapter": "carrier-pigeon" } }),
        ));
        assert!(
            matches!(
                &err,
                ConfigError::UnadvertisedLinkAdapter { link: l, adapter }
                    if *l == link && adapter == "carrier-pigeon"
            ),
            "{link}: {err:?}"
        );
    }
}

#[test]
fn an_empty_retention_class_rejects_the_document() {
    let err = load_err(&with_links(
        json!({ "send_only": { "retention_class": " " } }),
    ));
    assert!(
        matches!(err, ConfigError::EmptyLinkRetentionClass("send_only")),
        "{err:?}"
    );
}

#[test]
fn a_query_timeout_outside_its_range_rejects() {
    for bad in [0, 30_001] {
        let err = load_err(&with_links(json!({ "query": { "timeout_ms": bad } })));
        assert!(
            matches!(
                err,
                ConfigError::OutOfRange {
                    field: "links.query.timeout_ms",
                    ..
                }
            ),
            "{bad}: {err:?}"
        );
    }
    assert!(load(&with_links(json!({ "query": { "timeout_ms": 30_000 } }))).is_ok());
}

#[test]
fn the_control_link_is_always_on_and_takes_no_keys() {
    assert!(
        load(&with_links(json!({ "control": {} })))
            .unwrap()
            .links
            .is_default()
    );
    assert!(
        load(&with_links(json!({ "control": null })))
            .unwrap()
            .links
            .is_default()
    );
    for control in [json!({ "enabled": false }), json!({ "adapter": "iroh" })] {
        let doc = with_links(json!({ "control": control }));
        assert!(
            ConfigDocument::parse(&doc.to_string()).is_err(),
            "{control}"
        );
    }
}

#[test]
fn unknown_keys_fail_closed() {
    for links in [
        json!({ "gossip": {} }),
        json!({ "sync": { "class": "fully_trusted" } }),
        json!({ "query": { "per_peer_timeout_ms": 10 } }),
        json!({ "send_only": { "sync_scope": "full" } }),
    ] {
        let doc = with_links(links.clone());
        assert!(ConfigDocument::parse(&doc.to_string()).is_err(), "{links}");
    }
}

#[test]
fn the_digest_is_unchanged_without_links_or_with_only_the_defaults() {
    let plain = resolved_digest(&load(&base()).unwrap()).unwrap();
    let restated = resolved_digest(
        &load(&with_links(json!({
            "sync": { "enabled": false, "adapter": "iroh", "retention_class": "default" },
            "query": { "enabled": false },
            "send_only": { "enabled": false, "adapter": "iroh" },
            "control": {}
        })))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plain, restated);
    let resolved = load(&base()).unwrap();
    let canonical: Value = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert!(canonical.get("links").is_none());

    let on = resolved_digest(&load(&with_links(json!({ "sync": { "enabled": true } }))).unwrap())
        .unwrap();
    assert_ne!(plain, on);
}

#[test]
fn a_resolved_config_with_links_round_trips() {
    let resolved = load(&with_links(json!({
        "sync": { "enabled": true, "sync_scope": "full" },
        "query": { "enabled": true, "timeout_ms": 900 }
    })))
    .unwrap();
    let back: ResolvedConfig = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert_eq!(back, resolved);
}

#[test]
fn explain_lists_each_link_with_its_source() {
    let mut doc = with_links(json!({ "sync": { "enabled": true } }));
    doc["behavior"] = json!({ "sync_scope": "metadata" });
    let explanation = explain(&doc.to_string()).unwrap();
    let entry = |path: &str| {
        explanation
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no entry {path}"))
    };
    assert_eq!(entry("links.sync.enabled").value, "true");
    assert_eq!(entry("links.sync.enabled").source, "document");
    assert_eq!(entry("links.sync.adapter").source, "default");
    assert_eq!(entry("links.sync.sync_scope").value, "\"metadata\"");
    assert_eq!(entry("links.sync.sync_scope").source, "behavior.sync_scope");
    assert_eq!(entry("links.query.timeout_ms").value, "2000");
    assert_eq!(
        entry("links.query.timeout_ms").source,
        "fanout.per_peer_timeout_ms"
    );
    assert_eq!(entry("links.send_only.enabled").value, "false");
    assert_eq!(entry("links.control.enabled").value, "true");
    assert_eq!(entry("links.control.enabled").source, "fixed");
}

#[test]
fn links_is_behavior_and_the_schema_names_it() {
    assert_eq!(field_class("links"), Some(FieldClass::Behavior));
    assert_eq!(
        field_class("links.sync.adapter"),
        Some(FieldClass::Behavior)
    );
    let schema: Value =
        serde_json::from_str(include_str!("../config/id-config-v1.schema.json")).unwrap();
    assert_eq!(
        schema["properties"]["links"]["$ref"],
        json!("#/$defs/links")
    );
    let links = &schema["$defs"]["links"];
    assert_eq!(links["additionalProperties"], json!(false));
    assert_eq!(links["properties"]["control"]["properties"], json!({}));
    assert_eq!(
        schema["$defs"]["link_adapter"]["enum"],
        json!(["iroh", null])
    );
}
