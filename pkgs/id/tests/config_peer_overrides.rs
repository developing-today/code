//! Per-operation peer overrides `peers[].overrides`, sixth part of slice
//! 9111-03 (#9111 record §3.3, §6.2, §6.4, §6.5 step 4, Q9).
//!
//! Oracle: the record's rules, stated before the code. An override is
//! keyed by an operation the entry grants and may choose only a link
//! adapter this build advertises and a timeout within the per-peer limit.
//! It may not change class, gates or checks: those are not keys, so naming
//! one rejects the document. An override for an operation the entry does
//! not grant, or for `relay`, is a load error. An entry without
//! overrides, or with only empty ones, keeps its digest.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_config::{
    ConfigDocument, ConfigError, PeerOperation, ResolvedConfig, explain, resolved_digest,
};
use serde_json::{Value, json};

fn peer(overrides: Option<Value>) -> Value {
    let mut entry = json!({
        "peer_id": "alice",
        "class": "fully_trusted",
        "operations": ["sync.send", "query.out", "relay"],
        "relay_granted": true,
        "policy_version": 1
    });
    if let Some(overrides) = overrides {
        entry["overrides"] = overrides;
    }
    entry
}

fn doc(overrides: Option<Value>) -> Value {
    json!({
        "schema_version": 1,
        "config_id": "cfg-overrides",
        "revision": 1,
        "mode": "federated-relay",
        "protocol_versions": ["id/2.0"],
        "ceilings": {
            "max_hops": 5,
            "max_bytes": 1_048_576,
            "max_time_ms": 30_000,
            "max_fan_out": 16
        },
        "peers": [peer(overrides)]
    })
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
fn an_override_on_a_granted_operation_resolves() {
    let resolved = load(&doc(Some(json!({
        "query.out": { "timeout_ms": 700 },
        "sync.send": { "adapter": "iroh" }
    }))))
    .unwrap();
    let overrides = &resolved.peers[0].overrides;
    assert_eq!(overrides.len(), 2);
    assert_eq!(overrides[&PeerOperation::QueryOut].timeout_ms, Some(700));
    assert_eq!(overrides[&PeerOperation::QueryOut].adapter, None);
    assert_eq!(
        overrides[&PeerOperation::SyncSend].adapter.as_deref(),
        Some("iroh")
    );
}

#[test]
fn an_override_for_an_operation_not_granted_or_for_relay_rejects() {
    for (operation, rule_part) in [
        ("sync.accept", "does not grant"),
        ("query.answer", "does not grant"),
        ("relay", "relay takes no override"),
    ] {
        let err = load_err(&doc(Some(json!({ operation: { "timeout_ms": 500 } }))));
        assert!(
            matches!(
                &err,
                ConfigError::InvalidOperationOverride { peer_id, operation: op, rule }
                    if peer_id == "alice" && *op == operation && rule.contains(rule_part)
            ),
            "{operation}: {err:?}"
        );
    }
}

#[test]
fn an_override_can_choose_only_an_adapter_and_a_timeout() {
    for key in [
        "class",
        "relay_granted",
        "signature_verification",
        "replay_protection",
        "tenant_scope",
        "scope",
    ] {
        let value = doc(Some(json!({ "query.out": { key: true } })));
        assert!(ConfigDocument::parse(&value.to_string()).is_err(), "{key}");
    }
    let value = doc(Some(json!({ "query.later": { "timeout_ms": 5 } })));
    assert!(ConfigDocument::parse(&value.to_string()).is_err());
}

#[test]
fn an_unadvertised_adapter_rejects() {
    let err = load_err(&doc(Some(
        json!({ "sync.send": { "adapter": "smoke-signal" } }),
    )));
    assert!(
        matches!(
            &err,
            ConfigError::UnadvertisedOverrideAdapter { peer_id, operation: "sync.send", adapter }
                if peer_id == "alice" && adapter == "smoke-signal"
        ),
        "{err:?}"
    );
}

#[test]
fn a_timeout_outside_the_per_peer_limit_rejects() {
    for bad in [0, 30_001] {
        let err = load_err(&doc(Some(json!({ "query.out": { "timeout_ms": bad } }))));
        assert!(
            matches!(
                err,
                ConfigError::OutOfRange {
                    field: "peers[].overrides.timeout_ms",
                    ..
                }
            ),
            "{bad}: {err:?}"
        );
    }
}

#[test]
fn no_overrides_or_empty_ones_keep_the_digest() {
    let plain = resolved_digest(&load(&doc(None)).unwrap()).unwrap();
    let empty_map = resolved_digest(&load(&doc(Some(json!({})))).unwrap()).unwrap();
    let empty_entry = resolved_digest(
        &load(&doc(Some(json!({
            "query.out": {},
            "sync.send": { "adapter": null, "timeout_ms": null }
        }))))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plain, empty_map);
    assert_eq!(plain, empty_entry);
    let set =
        resolved_digest(&load(&doc(Some(json!({ "query.out": { "timeout_ms": 700 } })))).unwrap())
            .unwrap();
    assert_ne!(plain, set);
}

#[test]
fn a_resolved_config_with_overrides_round_trips() {
    let resolved = load(&doc(Some(json!({
        "query.out": { "timeout_ms": 700, "adapter": "iroh" }
    }))))
    .unwrap();
    let back: ResolvedConfig = serde_json::from_str(&resolved.canonical_json().unwrap()).unwrap();
    assert_eq!(back, resolved);
}

#[test]
fn explain_lists_each_override() {
    let value = doc(Some(json!({ "query.out": { "timeout_ms": 700 } })));
    let explanation = explain(&value.to_string()).unwrap();
    let entry = explanation
        .entries
        .iter()
        .find(|e| e.path == "peers[\"alice\"].overrides.query.out.timeout_ms")
        .unwrap();
    assert_eq!(entry.value, "700");
    assert_eq!(entry.source, "document");
    assert!(
        !explanation
            .entries
            .iter()
            .any(|e| e.path.ends_with("overrides.query.out.adapter"))
    );
}

#[test]
fn the_schema_lists_the_overridable_operations() {
    let schema: Value =
        serde_json::from_str(include_str!("../config/id-config-v1.schema.json")).unwrap();
    let overrides = &schema["$defs"]["peer"]["properties"]["overrides"];
    assert_eq!(overrides["additionalProperties"], json!(false));
    let mut keys: Vec<&String> = overrides["properties"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["query.answer", "query.out", "sync.accept", "sync.send"]
    );
    let over = &schema["$defs"]["operation_override"];
    assert_eq!(over["additionalProperties"], json!(false));
    assert_eq!(over["properties"]["timeout_ms"]["maximum"], json!(30_000));
}
